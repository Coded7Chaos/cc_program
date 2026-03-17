use std::sync::Arc;
use tauri::{AppHandle, State};
use crate::state::{AppState, ActiveTransfer};
use crate::transfer::sender;

/// Inicia la transferencia de un archivo a los peers especificados.
/// Retorna el transfer_id asignado.
#[tauri::command]
pub async fn send_file(
    path: String,
    dest_path: String,
    target_peer_ids: Vec<String>,
    state: State<'_, Arc<AppState>>,
    app_handle: AppHandle,
) -> Result<String, String> {
    tracing::info!("--- COMANDO SEND_FILE RECIBIDO ---");
    tracing::info!("Ruta origen: {}", path);
    tracing::info!("Ruta destino: {}", dest_path);
    tracing::info!("Peers objetivo: {:?}", target_peer_ids);

    if target_peer_ids.is_empty() {
        tracing::error!("Error: Lista de peers vacía");
        return Err("Debe seleccionar al menos un peer destino".to_string());
    }

    let state_clone = Arc::clone(&state);

    sender::start_send(path, dest_path, target_peer_ids, state_clone, app_handle)
        .await
        .map_err(|e| {
            tracing::error!("Error al iniciar transferencia: {}", e);
            e.to_string()
        })
}

/// Retorna todas las transferencias activas
#[tauri::command]
pub async fn get_active_transfers(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<ActiveTransfer>, String> {
    let transfers: Vec<ActiveTransfer> = state.active_transfers
        .iter()
        .map(|entry| entry.value().clone())
        .collect();
    Ok(transfers)
}

/// Cancela una transferencia activa
#[tauri::command]
pub async fn cancel_transfer(
    transfer_id: String,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    if let Some(cancel_tx) = state.cancel_senders.get(&transfer_id) {
        cancel_tx.send(true).map_err(|e| e.to_string())?;
    }
    Ok(())
}
