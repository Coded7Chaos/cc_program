//! Receptor TCP (transferencia).
//! - TCP: escucha conexiones entrantes de transferencia.

use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tauri::AppHandle;
use tracing::{error, info, warn};

use crate::transfer::receiver;
use crate::state::AppState;

/// Tarea que escucha conexiones TCP entrantes de transferencia.
/// Intenta el puerto configurado y si falla, busca el siguiente disponible.
pub async fn run_tcp_listener(
    state: Arc<AppState>,
    app_handle: AppHandle,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let mut port = *state.tcp_port.read().await;
    let max_retries = 100;

    let listener = loop {
        match TcpListener::bind(format!("0.0.0.0:{}", port)).await {
            Ok(l) => {
                // Actualizar el puerto real en el estado
                let mut p_lock = state.tcp_port.write().await;
                *p_lock = port;
                drop(p_lock);
                break l;
            }
            Err(e) => {
                warn!("Puerto {} ocupado, probando siguiente ({})...", port, e);
                port += 1;
                if port > 47833 + max_retries {
                    error!("No se pudo encontrar ningún puerto TCP disponible.");
                    return;
                }
            }
        }
    };

    info!("Listener TCP iniciado en puerto {}", port);

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, addr)) => {
                        info!("Conexión TCP entrante desde {}", addr);
                        let state_clone = state.clone();
                        let app_handle_clone = app_handle.clone();
                        tokio::spawn(async move {
                            if let Err(e) = receiver::handle_incoming_connection(
                                stream,
                                state_clone,
                                app_handle_clone,
                            ).await {
                                error!("Error manejando conexión TCP: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("Error aceptando conexión TCP: {}", e);
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!("Listener TCP: shutdown recibido");
                break;
            }
        }
    }
}
