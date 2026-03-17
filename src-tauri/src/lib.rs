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
    // Inicializar tracing: escribe en consola Y en archivo de log.
    // El archivo se guarda en %APPDATA%\com.NMFCC.app\logs\app.log (Windows)
    // o ~/Library/Logs/com.NMFCC.app/app.log (macOS).
    // Útil para diagnosticar problemas en máquinas receptoras sin acceso al terminal.
    init_logging();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        // Autostart: permite que la app arranque con el sistema operativo.
        // En Windows usa el registro (HKCU\...\Run), en macOS usa LaunchAgent.
        // No se habilita automáticamente; el usuario lo activa desde Configuración.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
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
            commands::config::get_autostart,
            commands::config::set_autostart,
            commands::peers::get_peers,
            commands::peers::refresh_peers,
            commands::peers::check_peers_online,
            commands::peers::clear_peers,
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

/// Configura el sistema de logging para escribir en consola y en archivo.
///
/// Ruta del archivo:
///   Windows : %APPDATA%\com.NMFCC.app\logs\app.log
///   macOS   : ~/Library/Logs/com.NMFCC.app/app.log
///   Fallback: app.log (directorio de trabajo)
///
/// El archivo se rota automáticamente cuando supera 5 MB para no llenar el disco.
/// Para ver los logs en un receptor: abrir el Bloc de Notas y navegar a esa ruta.
fn init_logging() {
    use tracing_subscriber::fmt::writer::MakeWriterExt;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let log_path = get_log_path();

    // Rotar si supera 5 MB
    if let Some(ref p) = log_path {
        if std::fs::metadata(p).map(|m| m.len()).unwrap_or(0) > 5 * 1024 * 1024 {
            let _ = std::fs::remove_file(p);
        }
    }

    let file_writer = log_path.as_deref().and_then(|p| {
        if let Some(parent) = std::path::Path::new(p).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
            .map(std::sync::Mutex::new)
    });

    match file_writer {
        Some(fw) => {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false) // Sin colores ANSI en el archivo
                .with_writer(std::io::stdout.and(fw))
                .init();
            if let Some(ref p) = log_path {
                eprintln!("[P2P Deployer] Logs guardados en: {}", p);
            }
        }
        None => {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .init();
        }
    }
}

/// Devuelve la ruta del archivo de log según el sistema operativo.
fn get_log_path() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA").ok().map(|appdata| {
            format!("{}\\com.NMFCC.app\\logs\\app.log", appdata)
        })
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME").ok().map(|home| {
            format!("{}/Library/Logs/com.NMFCC.app/app.log", home)
        })
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Some("app.log".to_string())
    }
}

/// Detecta la IP local del sistema priorizando la interfaz LAN real.
///
/// Orden de prioridad:
///   1. Redes privadas de clase A/B/C típicas de LAN corporativa (10.x, 172.16-31.x, 192.168.x)
///      → se prefiere la que tenga más octetos "normales" de LAN universitaria
///   2. Cualquier otra interfaz no-loopback, no-APIPA, no-virtual (descarte de rangos de VM)
///
/// Problema que resuelve: en Windows, VMware/VirtualBox crean adaptadores virtuales
/// (192.168.56.x, 172.x.x.x) que suelen aparecer ANTES de la Ethernet real en la lista.
/// Si se usa la IP del adaptador virtual como "local_ip", los receptores intentan
/// descargar chunks de una IP inalcanzable y la transferencia falla silenciosamente.
fn get_local_ip() -> Option<String> {
    let addrs = if_addrs::get_if_addrs().ok()?;

    let mut candidates: Vec<std::net::Ipv4Addr> = Vec::new();

    for iface in &addrs {
        if iface.is_loopback() {
            continue;
        }
        if let std::net::IpAddr::V4(ip) = iface.ip() {
            let o = ip.octets();
            // Ignorar APIPA (169.254.x.x)
            if o[0] == 169 && o[1] == 254 {
                continue;
            }
            // Ignorar rango de VMware host-only / VirtualBox host-only más comunes:
            //   192.168.56.x  → VirtualBox default host-only
            //   192.168.99.x  → Docker Machine / VirtualBox legacy
            //   172.17-19.x   → Docker bridge
            if o[0] == 192 && o[1] == 168 && (o[2] == 56 || o[2] == 99) {
                continue;
            }
            if o[0] == 172 && o[1] >= 17 && o[1] <= 19 {
                continue;
            }
            candidates.push(ip);
        }
    }

    // Preferir 192.168.x.x (más común en LAN universitaria) primero,
    // luego 10.x.x.x, luego 172.16-31.x, luego cualquier otra.
    let score = |ip: &std::net::Ipv4Addr| -> u8 {
        let o = ip.octets();
        if o[0] == 192 && o[1] == 168 { 0 }
        else if o[0] == 10 { 1 }
        else if o[0] == 172 && o[1] >= 16 && o[1] <= 31 { 2 }
        else { 3 }
    };
    candidates.sort_by_key(score);

    candidates.first().map(|ip| ip.to_string())
}
