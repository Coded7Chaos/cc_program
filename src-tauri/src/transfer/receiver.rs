//! Receiver: acepta TransferAnnounce, descarga chunks y escribe al disco.

use std::path::Path;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::io::BufStream;
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tracing::{error, info, warn};

use crate::protocol::codec::{read_typed_frame, write_frame};
use crate::protocol::messages::{
    ChunkRequest, TcpMsgType, TransferAccepted, TransferAnnounce,
    TransferRejected,
};
use crate::state::{ActiveTransfer, AppState, TransferRole, TransferStatus};
use crate::transfer::chunker::{
    preallocate_file, sha1_file, verify_chunk, write_chunk_at_offset,
};

fn current_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Punto de entrada desde el TCP listener: maneja la conexión entrante
pub async fn handle_incoming_connection(
    stream: TcpStream,
    state: Arc<AppState>,
    app_handle: AppHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut buf_stream = BufStream::new(stream);

    // Leer el frame completo
    let (announce, _data): (TransferAnnounce, _) = read_typed_frame(&mut buf_stream).await?;

    info!(
        "TransferAnnounce recibido: {} ({} chunks) de {}",
        announce.file_name, announce.total_chunks, announce.sender_ip
    );

    // Verificar espacio disponible en disco (básico)
    let dest_path = Path::new(&announce.destination_path).join(&announce.file_name);

    // Auto-aceptar según configuración
    let auto_accept = state.config.read().await.auto_accept_transfers;
    if !auto_accept {
        let rejected = TransferRejected {
            msg_type: TcpMsgType::TransferRejected,
            transfer_id: announce.transfer_id.clone(),
            peer_id: state.peer_id.clone(),
            reason: "Auto-accept deshabilitado".to_string(),
        };
        write_frame(&mut buf_stream, &rejected, &[]).await?;
        return Ok(());
    }

    // Registrar transferencia
    let active = ActiveTransfer {
        transfer_id: announce.transfer_id.clone(),
        file_name: announce.file_name.clone(),
        file_path: dest_path.to_string_lossy().to_string(),
        file_size: announce.file_size,
        total_chunks: announce.total_chunks,
        chunk_size: announce.chunk_size,
        chunk_hashes: announce.chunk_hashes.clone(),
        destination_path: announce.destination_path.clone(),
        role: TransferRole::Receiver,
        chunks_done: vec![false; announce.total_chunks as usize],
        status: TransferStatus::InProgress,
        target_peers: vec![],
        sender_ip: announce.sender_ip.clone(),
        sender_peer_id: announce.sender_peer_id.clone(),
        bytes_transferred: 0,
        started_at: current_epoch(),
    };
    state
        .active_transfers
        .insert(announce.transfer_id.clone(), active.clone());

    // Emitir evento de transferencia entrante
    if let Err(e) = app_handle.emit("transfer-incoming", &active) {
        error!("Error emitiendo transfer-incoming: {}", e);
    }

    // Cancelation channel
    let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);
    state
        .cancel_senders
        .insert(announce.transfer_id.clone(), cancel_tx);

    // Aceptar
    let accepted = TransferAccepted {
        msg_type: TcpMsgType::TransferAccepted,
        transfer_id: announce.transfer_id.clone(),
        peer_id: state.peer_id.clone(),
    };
    write_frame(&mut buf_stream, &accepted, &[]).await?;

    // Pre-allocar archivo destino
    if let Err(e) = preallocate_file(&dest_path, announce.file_size) {
        error!("Error pre-allocando archivo: {}", e);
        // Continuar de todas formas - write_chunk_at_offset crea el archivo
    }

    // Semáforo para máx 4 chunks concurrentes
    let config = state.config.read().await;
    let max_concurrent = config.max_concurrent_chunks;
    drop(config);
    let semaphore = Arc::new(Semaphore::new(max_concurrent));

    let mut last_progress = Instant::now();
    let progress_interval = std::time::Duration::from_millis(250);

    // Descargar chunks secuencialmente (el sender sirve bajo demanda)
    // Para P2P swarm: podríamos pedir chunks en paralelo a múltiples peers.
    // Por ahora: pedimos todos al sender original.
    for chunk_index in 0..announce.total_chunks {
        // Verificar cancelación
        if *cancel_rx.borrow() {
            info!("Transferencia {} cancelada", announce.transfer_id);
            if let Some(mut t) = state.active_transfers.get_mut(&announce.transfer_id) {
                t.status = TransferStatus::Cancelled;
            }
            let _ = app_handle.emit("transfer-error", serde_json::json!({
                "transfer_id": announce.transfer_id,
                "error": "Cancelado"
            }));
            return Ok(());
        }

        let _permit = semaphore.clone().acquire_owned().await?;

        // Pedir chunk
        let request = ChunkRequest {
            msg_type: TcpMsgType::ChunkRequest,
            transfer_id: announce.transfer_id.clone(),
            chunk_index,
            requester_id: state.peer_id.clone(),
        };
        write_frame(&mut buf_stream, &request, &[]).await?;

        // Recibir respuesta
        let (response, chunk_data): (crate::protocol::messages::ChunkResponse, _) =
            read_typed_frame(&mut buf_stream).await?;

        if response.transfer_id != announce.transfer_id {
            warn!("Transfer ID inesperado en ChunkResponse");
            continue;
        }

        // Verificar integridad
        let expected_hash = announce
            .chunk_hashes
            .get(chunk_index as usize)
            .cloned()
            .unwrap_or_default();

        if let Err(e) = verify_chunk(&chunk_data, &expected_hash) {
            error!("Hash mismatch en chunk {}: {}", chunk_index, e);
            if let Some(mut t) = state.active_transfers.get_mut(&announce.transfer_id) {
                t.status = TransferStatus::Failed(format!("Hash mismatch chunk {}", chunk_index));
            }
            let _ = app_handle.emit("transfer-error", serde_json::json!({
                "transfer_id": announce.transfer_id,
                "error": format!("Hash mismatch en chunk {}", chunk_index)
            }));
            return Ok(());
        }

        // Escribir chunk al disco
        write_chunk_at_offset(&dest_path, chunk_index, announce.chunk_size, &chunk_data)?;

        // Actualizar estado
        if let Some(mut transfer) = state.active_transfers.get_mut(&announce.transfer_id) {
            if let Some(done) = transfer.chunks_done.get_mut(chunk_index as usize) {
                *done = true;
            }
            transfer.bytes_transferred += chunk_data.len() as u64;

            if last_progress.elapsed() >= progress_interval {
                let chunks_done = transfer.chunks_completed();
                let bytes = transfer.bytes_transferred;
                drop(transfer);
                let _ = app_handle.emit(
                    "transfer-progress",
                    serde_json::json!({
                        "transfer_id": announce.transfer_id,
                        "chunks_completed": chunks_done,
                        "total_chunks": announce.total_chunks,
                        "bytes_transferred": bytes,
                        "speed_bps": 0,
                        "status": "InProgress"
                    }),
                );
                last_progress = Instant::now();
            }
        }
    }

    // Verificar SHA1 completo del archivo
    let transfer_id = announce.transfer_id.clone();
    let dest_path_clone = dest_path.clone();
    let final_sha1 = tokio::task::spawn_blocking(move || sha1_file(&dest_path_clone))
        .await??;

    info!(
        "Transferencia {} completada. SHA1: {}",
        transfer_id, final_sha1
    );

    // Actualizar estado
    if let Some(mut t) = state.active_transfers.get_mut(&transfer_id) {
        t.status = TransferStatus::Completed;
        // Marcar todos los chunks como completados
        for done in t.chunks_done.iter_mut() {
            *done = true;
        }
    }

    // Emitir completado
    let _ = app_handle.emit("transfer-complete", &transfer_id);

    // Emitir progreso final
    let _ = app_handle.emit(
        "transfer-progress",
        serde_json::json!({
            "transfer_id": transfer_id,
            "chunks_completed": announce.total_chunks,
            "total_chunks": announce.total_chunks,
            "bytes_transferred": announce.file_size,
            "speed_bps": 0,
            "status": "Completed"
        }),
    );

    Ok(())
}
