use std::sync::Arc;
use tauri::State;
use crate::state::AppState;

/// Retorna el puerto TCP en el que escucha la aplicación
#[tauri::command]
pub async fn get_app_tcp_port(state: State<'_, Arc<AppState>>) -> Result<u16, String> {
    Ok(*state.tcp_port.read().await)
}

/// Retorna el Peer ID de esta instancia
#[tauri::command]
pub fn get_local_peer_id(state: State<'_, Arc<AppState>>) -> String {
    state.peer_id.clone()
}
