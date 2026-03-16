use std::sync::Arc;
use tauri::State;
use crate::state::{AppState, PeerEntry};
use crate::network::discovery::send_announce;
use crate::network::scanner;
use tauri::AppHandle;

/// Retorna todos los peers conocidos
#[tauri::command]
pub async fn get_peers(state: State<'_, Arc<AppState>>) -> Result<Vec<PeerEntry>, String> {
    let peers: Vec<PeerEntry> = state.peers
        .iter()
        .map(|entry| entry.value().clone())
        .collect();
    Ok(peers)
}

/// Dispara un broadcast UDP inmediato y lanza un escaneo de subnet
#[tauri::command]
pub async fn refresh_peers(
    state: State<'_, Arc<AppState>>,
    app_handle: AppHandle,
) -> Result<(), String> {
    // Broadcast inmediato
    send_announce(&state).map_err(|e| e.to_string())?;

    // Lanzar escaneo en background
    let state_clone = Arc::clone(&state);
    let app_handle_clone = app_handle.clone();
    let shutdown_rx = state.shutdown_tx.subscribe();
    tokio::spawn(async move {
        scanner::run_subnet_scan(state_clone, app_handle_clone, shutdown_rx).await;
    });

    Ok(())
}
