mod state;
mod protocol;
mod network;
mod transfer;
mod commands;

use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, RunEvent,
};
use tokio::sync::broadcast;
use tracing::info;
use uuid::Uuid;

use network::listener;
use state::AppState;

pub fn run() {
    // Inicializar tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // Si ya hay una instancia corriendo, mostrar la ventana
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let app_handle = app.handle().clone();

            // Inicializar runtime de tokio para las tareas async
            let rt = tokio::runtime::Runtime::new().expect("No se pudo crear Tokio runtime");

            // Obtener o generar peer_id persistido
            let peer_id = {
                // Por simplicidad en v1, generamos un UUID fresco cada vez.
                // En producción, se persistiría con tauri-plugin-store.
                Uuid::new_v4().to_string()
            };

            let hostname = gethostname::gethostname()
                .to_string_lossy()
                .to_string();

            let local_ip = get_local_ip().unwrap_or_else(|| "127.0.0.1".to_string());

            info!("Iniciando P2P File Deployer");
            info!("peer_id: {}", peer_id);
            info!("hostname: {}", hostname);
            info!("local_ip: {}", local_ip);

            let (shutdown_tx, _) = broadcast::channel::<()>(4);

            let state = Arc::new(AppState::new(
                peer_id,
                hostname,
                local_ip,
                shutdown_tx.clone(),
            ));

            app.manage(state.clone());

            // Lanzar tareas de red en background
            {
                let state_clone = state.clone();
                let app_handle_clone = app_handle.clone();
                let shutdown_rx2 = shutdown_tx.subscribe();

                rt.spawn(async move {
                    // Escuchar solo conexiones TCP (transferencia y señal de vida)
                    tokio::join!(
                        listener::run_tcp_listener(state_clone.clone(), app_handle_clone.clone(), shutdown_rx2),
                    );
                });
            }

            // Configurar System Tray
            let tray_menu = Menu::with_items(
                &app_handle,
                &[
                    &MenuItem::with_id(&app_handle, "show", "Abrir", true, None::<&str>)?,
                    &MenuItem::with_id(&app_handle, "quit", "Salir", true, None::<&str>)?,
                ],
            )?;

            let _tray = TrayIconBuilder::new()
                .menu(&tray_menu)
                .tooltip("P2P File Deployer")
                .on_menu_event(move |app, event| {
                    match event.id.as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => {
                            // Señal de shutdown graceful
                            let _ = shutdown_tx.send(());
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(&app_handle)?;

            // Guardar el runtime para que no se destruya
            app.manage(rt);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::config::get_local_peer_id,
            commands::config::get_app_tcp_port,
            commands::peers::get_peers,
            commands::peers::refresh_peers,
            commands::files::open_file_dialog,
            commands::files::get_file_info,
            commands::transfers::send_file,
            commands::transfers::get_active_transfers,
            commands::transfers::cancel_transfer,
        ])
        .on_window_event(|window, event| {
            // Minimizar a tray en lugar de cerrar
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                window.hide().unwrap_or_default();
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!())
        .expect("Error al construir la aplicación Tauri")
        .run(|_app_handle, event| {
            if let RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
            }
        });
}

/// Detecta la IP local del sistema (primera interfaz no-loopback)
fn get_local_ip() -> Option<String> {
    let addrs = if_addrs::get_if_addrs().ok()?;
    for iface in addrs {
        if iface.is_loopback() {
            continue;
        }
        if let std::net::IpAddr::V4(ip) = iface.ip() {
            let octets = ip.octets();
            // Ignorar APIPA (169.254.x.x)
            if octets[0] == 169 && octets[1] == 254 {
                continue;
            }
            return Some(ip.to_string());
        }
    }
    None
}
