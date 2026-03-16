//! Receptor TCP (transferencia).
//! - TCP: escucha conexiones entrantes de transferencia en puerto 47833

use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tauri::AppHandle;
use tracing::{error, info};

use crate::network::discovery::TCP_PORT;
use crate::transfer::receiver;

/// Tarea que escucha conexiones TCP entrantes de transferencia
pub async fn run_tcp_listener(
    state: Arc<AppState>,
    app_handle: AppHandle,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let listener = match TcpListener::bind(format!("0.0.0.0:{}", TCP_PORT)).await {
        Ok(l) => l,
        Err(e) => {
            error!("No se pudo iniciar TCP listener en puerto {}: {}", TCP_PORT, e);
            return;
        }
    };

    info!("Listener TCP iniciado en puerto {}", TCP_PORT);

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

use crate::state::AppState;
