use std::sync::Arc;
use std::time::Duration;
use tauri::State;
use crate::state::{AppState, PeerEntry};
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

/// Verifica en tiempo real qué peers responden a una conexión TCP.
/// Retorna la lista de peer_ids que están efectivamente disponibles en este momento.
#[tauri::command]
pub async fn check_peers_online(
    peer_ids: Vec<String>,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<String>, String> {
    tracing::info!("[check_peers_online] Verificando {} peers en tiempo real...", peer_ids.len());
    let mut handles = Vec::new();

    for peer_id in &peer_ids {
        // Obtener la dirección sin mantener el lock durante la operación async
        let (addr, ip) = match state.peers.get(peer_id) {
            Some(peer) => (format!("{}:{}", peer.ip, peer.tcp_port), peer.ip.clone()),
            None => {
                tracing::warn!("[check_peers_online] Peer ID '{}' no está en el mapa de peers", peer_id);
                continue;
            }
        };

        let peer_id_clone = peer_id.clone();
        let handle = tokio::spawn(async move {
            // Intento de conexión TCP con timeout de 2 segundos
            match tokio::time::timeout(
                Duration::from_secs(2),
                tokio::net::TcpStream::connect(&addr),
            ).await {
                Ok(Ok(_)) => {
                    tracing::debug!("[check_peers_online] {} ({}) — ONLINE", peer_id_clone, ip);
                    Some(peer_id_clone)
                }
                Ok(Err(e)) => {
                    tracing::debug!("[check_peers_online] {} ({}) — OFFLINE: {}", peer_id_clone, ip, e);
                    None
                }
                Err(_) => {
                    tracing::debug!("[check_peers_online] {} ({}) — TIMEOUT (2s)", peer_id_clone, ip);
                    None
                }
            }
        });
        handles.push(handle);
    }

    let mut online_ids = Vec::new();
    for handle in handles {
        if let Ok(Some(peer_id)) = handle.await {
            online_ids.push(peer_id);
        }
    }

    tracing::info!(
        "[check_peers_online] Resultado: {}/{} peers responden.",
        online_ids.len(),
        peer_ids.len()
    );
    Ok(online_ids)
}

/// Lanza un escaneo de subnet ARP inmediato
#[tauri::command]
pub async fn refresh_peers(
    state: State<'_, Arc<AppState>>,
    app_handle: AppHandle,
) -> Result<(), String> {
    // Lanzar escaneo en background
    let state_clone = Arc::clone(&state);
    let app_handle_clone = app_handle.clone();
    let shutdown_rx = state.shutdown_tx.subscribe();
    tokio::spawn(async move {
        scanner::run_subnet_scan(state_clone, app_handle_clone, shutdown_rx).await;
    });

    Ok(())
}
