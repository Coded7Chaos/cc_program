//! Sender: inicia transferencias y actúa como la semilla (seed) inicial del enjambre.

use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;
use tokio::net::TcpStream;
use tokio::io::BufStream;
use tracing::info;
use uuid::Uuid;

use crate::protocol::codec::write_frame;
use crate::protocol::messages::{
    SwarmPeer, TcpMsgType, TransferAnnounce,
};
use crate::state::{ActiveTransfer, AppState, TransferRole, TransferStatus};
use crate::transfer::chunker::{
    build_chunk_map, chunk_count, CHUNK_SIZE,
};

fn current_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Inicia el proceso de envío de un archivo creando un enjambre P2P.
pub async fn start_send(
    file_path: String,
    dest_path: String,
    target_peer_ids: Vec<String>,
    state: Arc<AppState>,
    _app_handle: AppHandle,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let path = Path::new(&file_path);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("archivo")
        .to_string();
    let file_size = std::fs::metadata(path)?.len();
    let total_chunks = chunk_count(file_size, CHUNK_SIZE);

    info!("Preparando enjambre para {} ({} chunks)...", file_name, total_chunks);

    // 1. Calcular hashes
    let path_clone = path.to_path_buf();
    let chunk_hashes = tokio::task::spawn_blocking(move || build_chunk_map(&path_clone, CHUNK_SIZE))
        .await??;

    let transfer_id = Uuid::new_v4().to_string();

    // 2. Construir la lista del Enjambre (Swarm)
    let mut swarm = Vec::new();
    // Añadirse a sí mismo como el primer peer (la semilla)
    swarm.push(SwarmPeer {
        peer_id: state.peer_id.clone(),
        ip: state.local_ip.clone(),
        tcp_port: 47833, // Puerto estándar de la app
    });

    for peer_id in &target_peer_ids {
        if let Some(peer) = state.peers.get(peer_id) {
            swarm.push(SwarmPeer {
                peer_id: peer_id.clone(),
                ip: peer.ip.clone(),
                tcp_port: peer.tcp_port,
            });
        }
    }

    // 3. Registrar en el Tracker local que nosotros tenemos todo
    {
        let mut tracker = state.tracker.lock().await;
        let entry = tracker.get_or_create(&transfer_id);
        entry.set_swarm(swarm.clone());
        for i in 0..total_chunks {
            entry.add_peer_chunk(state.peer_id.clone(), i);
        }
    }

    // 4. Registrar transferencia en el estado
    let active = ActiveTransfer {
        transfer_id: transfer_id.clone(),
        file_name: file_name.clone(),
        file_path: file_path.clone(),
        file_size,
        total_chunks,
        chunk_size: CHUNK_SIZE,
        chunk_hashes: chunk_hashes.clone(),
        destination_path: dest_path.clone(),
        role: TransferRole::Sender,
        chunks_done: vec![true; total_chunks as usize], // El sender ya tiene todo
        status: TransferStatus::InProgress,
        target_peers: target_peer_ids.clone(),
        swarm: swarm.clone(),
        sender_ip: state.local_ip.clone(),
        sender_peer_id: state.peer_id.clone(),
        bytes_transferred: 0,
        started_at: current_epoch(),
    };
    state.active_transfers.insert(transfer_id.clone(), active);

    // 5. Notificar a todos los peers para que se unan al enjambre
    for peer_id in target_peer_ids {
        let peer = match state.peers.get(&peer_id) {
            Some(p) => p.clone(),
            None => continue,
        };

        let announce = TransferAnnounce {
            msg_type: TcpMsgType::TransferAnnounce,
            transfer_id: transfer_id.clone(),
            sender_peer_id: state.peer_id.clone(),
            sender_ip: state.local_ip.clone(),
            file_name: file_name.clone(),
            file_size,
            total_chunks,
            chunk_size: CHUNK_SIZE,
            chunk_hashes: chunk_hashes.clone(),
            destination_path: dest_path.clone(),
            swarm: swarm.clone(),
        };

        tokio::spawn(async move {
            if let Ok(stream) = TcpStream::connect(format!("{}:{}", peer.ip, peer.tcp_port)).await {
                let mut buf = BufStream::new(stream);
                let _ = write_frame(&mut buf, &announce, &[]).await;
            }
        });
    }

    Ok(transfer_id)
}
