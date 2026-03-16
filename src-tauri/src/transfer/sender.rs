//! Sender: inicia transferencias y sirve chunks por TCP a los receptores.

use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::net::TcpStream;
use tokio::io::BufStream;
use tracing::{error, info};
use uuid::Uuid;

use crate::protocol::codec::{read_typed_frame, write_frame};
use crate::protocol::messages::{
    ChunkRequest, ChunkResponse, TcpMsgEnvelope, TcpMsgType, TransferAnnounce, TransferComplete,
    TransferError as TransferErrorMsg,
};
use crate::state::{ActiveTransfer, AppState, TransferRole, TransferStatus};
use crate::transfer::chunker::{
    build_chunk_map, chunk_count, preallocate_file, read_chunk, sha1_file, CHUNK_SIZE,
};

fn current_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Inicia el proceso de envío de un archivo a múltiples peers.
/// Retorna el transfer_id asignado.
pub async fn start_send(
    file_path: String,
    dest_path: String,
    target_peer_ids: Vec<String>,
    state: Arc<AppState>,
    app_handle: AppHandle,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let path = Path::new(&file_path);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("archivo")
        .to_string();
    let file_size = std::fs::metadata(path)?.len();
    let total_chunks = chunk_count(file_size, CHUNK_SIZE);

    info!(
        "Calculando hashes para {} ({} chunks)...",
        file_name, total_chunks
    );

    // Calcular hashes de chunks en un thread bloqueante
    let path_clone = path.to_path_buf();
    let chunk_hashes = tokio::task::spawn_blocking(move || build_chunk_map(&path_clone, CHUNK_SIZE))
        .await??;

    let transfer_id = Uuid::new_v4().to_string();

    // Registrar transferencia en el estado
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
        chunks_done: vec![false; total_chunks as usize],
        status: TransferStatus::InProgress,
        target_peers: target_peer_ids.clone(),
        sender_ip: state.local_ip.clone(),
        sender_peer_id: state.peer_id.clone(),
        bytes_transferred: 0,
        started_at: current_epoch(),
    };
    state.active_transfers.insert(transfer_id.clone(), active);

    // Crear canal de cancelación
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    state.cancel_senders.insert(transfer_id.clone(), cancel_tx);

    // Conectar y enviar a cada peer en paralelo
    for peer_id in target_peer_ids {
        let peer = match state.peers.get(&peer_id) {
            Some(p) => p.clone(),
            None => {
                error!("Peer no encontrado: {}", peer_id);
                continue;
            }
        };

        let transfer_id_clone = transfer_id.clone();
        let file_path_clone = file_path.clone();
        let dest_path_clone = dest_path.clone();
        let chunk_hashes_clone = chunk_hashes.clone();
        let file_name_clone = file_name.clone();
        let state_clone = state.clone();
        let app_handle_clone = app_handle.clone();
        let cancel_rx_clone = cancel_rx.clone();

        tokio::spawn(async move {
            if let Err(e) = send_to_peer(
                &peer.ip,
                peer.tcp_port,
                &transfer_id_clone,
                &file_path_clone,
                &file_name_clone,
                file_size,
                total_chunks,
                &dest_path_clone,
                chunk_hashes_clone,
                state_clone,
                app_handle_clone,
                cancel_rx_clone,
            )
            .await
            {
                error!("Error enviando a {}: {}", peer.ip, e);
            }
        });
    }

    Ok(transfer_id)
}

/// Maneja el envío completo a un peer específico
async fn send_to_peer(
    peer_ip: &str,
    peer_port: u16,
    transfer_id: &str,
    file_path: &str,
    file_name: &str,
    file_size: u64,
    total_chunks: u32,
    dest_path: &str,
    chunk_hashes: Vec<String>,
    state: Arc<AppState>,
    app_handle: AppHandle,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = format!("{}:{}", peer_ip, peer_port);
    info!("Conectando a {} para transferencia {}", addr, transfer_id);

    let stream = TcpStream::connect(&addr).await?;
    let mut buf_stream = BufStream::new(stream);

    // Enviar TransferAnnounce
    let announce = TransferAnnounce {
        msg_type: crate::protocol::messages::TcpMsgType::TransferAnnounce,
        transfer_id: transfer_id.to_string(),
        sender_peer_id: state.peer_id.clone(),
        sender_ip: state.local_ip.clone(),
        file_name: file_name.to_string(),
        file_size,
        total_chunks,
        chunk_size: CHUNK_SIZE,
        chunk_hashes: chunk_hashes.clone(),
        destination_path: dest_path.to_string(),
    };
    write_frame(&mut buf_stream, &announce, &[]).await?;

    // Esperar aceptación o rechazo
    let (envelope, _): (TcpMsgEnvelope, _) = read_typed_frame(&mut buf_stream).await?;
    match envelope.msg_type {
        TcpMsgType::TransferRejected => {
            info!("Transferencia {} rechazada por {}", transfer_id, peer_ip);
            return Ok(());
        }
        TcpMsgType::TransferAccepted => {
            info!("Transferencia {} aceptada por {}", transfer_id, peer_ip);
        }
        other => {
            error!("Respuesta inesperada: {:?}", other);
            return Ok(());
        }
    }

    let path = Path::new(file_path);
    let mut last_progress_emit = std::time::Instant::now();
    let progress_interval = std::time::Duration::from_millis(250); // 4 Hz

    // Servir chunks bajo demanda
    loop {
        // Verificar cancelación
        if *cancel_rx.borrow() {
            let error_msg = TransferErrorMsg {
                msg_type: TcpMsgType::TransferError,
                transfer_id: transfer_id.to_string(),
                error: "Cancelado por el usuario".to_string(),
            };
            let _ = write_frame(&mut buf_stream, &error_msg, &[]).await;
            break;
        }

        let (request, _): (ChunkRequest, _) = match read_typed_frame(&mut buf_stream).await {
            Ok(r) => r,
            Err(_) => break,
        };

        if request.transfer_id != transfer_id {
            continue;
        }

        let hash = chunk_hashes
            .get(request.chunk_index as usize)
            .cloned()
            .unwrap_or_default();

        let chunk_data = match read_chunk(path, request.chunk_index, CHUNK_SIZE, &hash) {
            Ok(data) => data,
            Err(e) => {
                error!("Error leyendo chunk {}: {}", request.chunk_index, e);
                let error_msg = TransferErrorMsg {
                    msg_type: TcpMsgType::TransferError,
                    transfer_id: transfer_id.to_string(),
                    error: format!("Error leyendo chunk: {}", e),
                };
                let _ = write_frame(&mut buf_stream, &error_msg, &[]).await;
                break;
            }
        };

        let response = ChunkResponse {
            msg_type: TcpMsgType::ChunkResponse,
            transfer_id: transfer_id.to_string(),
            chunk_index: request.chunk_index,
            sha1_hash: hash.clone(),
            chunk_size: chunk_data.len() as u32,
        };
        write_frame(&mut buf_stream, &response, &chunk_data).await?;

        // Actualizar estado
        if let Some(mut transfer) = state.active_transfers.get_mut(transfer_id) {
            if let Some(done) = transfer.chunks_done.get_mut(request.chunk_index as usize) {
                *done = true;
            }
            transfer.bytes_transferred += chunk_data.len() as u64;

            // Emitir progreso throttled a 4 Hz
            if last_progress_emit.elapsed() >= progress_interval {
                let chunks_done = transfer.chunks_completed();
                let bytes = transfer.bytes_transferred;
                drop(transfer);
                let _ = app_handle.emit(
                    "transfer-progress",
                    serde_json::json!({
                        "transfer_id": transfer_id,
                        "chunks_completed": chunks_done,
                        "total_chunks": total_chunks,
                        "bytes_transferred": bytes,
                        "speed_bps": 0,
                        "status": "InProgress"
                    }),
                );
                last_progress_emit = std::time::Instant::now();
            }
        }

        // Verificar si el receptor pidió todos los chunks
        // (el receptor cierra la conexión cuando termina)
    }

    info!("Envío a {} completado", peer_ip);
    Ok(())
}
