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

/// Retorna si la app está configurada para arrancar con el sistema
#[tauri::command]
pub async fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch()
        .is_enabled()
        .map_err(|e| e.to_string())
}

/// Habilita o deshabilita el arranque automático con el sistema
#[tauri::command]
pub async fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let autolaunch = app.autolaunch();
    if enabled {
        autolaunch.enable().map_err(|e| e.to_string())
    } else {
        autolaunch.disable().map_err(|e| e.to_string())
    }
}
