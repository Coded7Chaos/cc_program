//! Receiver: maneja el protocolo BitTorrent P2P. 
//! - Acepta anuncios de transferencia.
//! - Se une al enjambre (swarm).
//! - Sirve chunks a otros peers mientras descarga.
//! - Notifica "HAVE" cuando completa un chunk.

use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::io::BufStream;
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};

use crate::protocol::codec::{read_typed_frame, write_frame};
use crate::protocol::messages::{
    ChunkRequest, ChunkResponse, HaveChunk, SwarmPeer, TcpMessage, TcpMsgType,
    TransferAccepted, TransferAnnounce,
};
use crate::state::{ActiveTransfer, AppState, TransferRole, TransferStatus};
use crate::transfer::chunker::{
    preallocate_file, read_chunk, verify_chunk, write_chunk_at_offset,
};

fn current_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Maneja una conexión TCP entrante (puede ser un anuncio, un request de chunk o un HAVE)
pub async fn handle_incoming_connection(
    stream: TcpStream,
    state: Arc<AppState>,
    app_handle: AppHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut buf_stream = BufStream::new(stream);
    
    // Leer el primer mensaje para identificar el propósito de la conexión
    let (msg, _data): (TcpMessage, _) = read_typed_frame(&mut buf_stream).await?;

    match msg {
        TcpMessage::TransferAnnounce(announce) => {
            info!("--- NUEVA TRANSFERENCIA ENTRANTE ---");
            handle_transfer_announce(announce, buf_stream, state, app_handle).await
        }
        TcpMessage::ChunkRequest(req) => {
            debug!("Solicitud de chunk recibida: idx {} de {}", req.chunk_index, req.requester_id);
            handle_chunk_request(req, buf_stream, state).await
        }
        TcpMessage::HaveChunk(have) => {
            debug!("Notificación HAVE recibida: chunk {} de {}", have.chunk_index, have.peer_id);
            handle_have_chunk(have, state).await
        }
        _ => {
            warn!("Mensaje TCP no reconocido o fuera de contexto");
            Ok(())
        }
    }
}

/// Procesa el anuncio inicial de una transferencia y arranca la descarga P2P
async fn handle_transfer_announce(
    announce: TransferAnnounce,
    mut stream: BufStream<TcpStream>,
    state: Arc<AppState>,
    app_handle: AppHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("╔══════════════════════════════════════╗");
    info!("║  NUEVA TRANSFERENCIA ENTRANTE (P2P)  ║");
    info!("╚══════════════════════════════════════╝");
    info!("[receiver] Archivo: '{}' | {} chunks | {} bytes", announce.file_name, announce.total_chunks, announce.file_size);
    info!("[receiver] Origen: {} (peer_id: {})", announce.sender_ip, announce.sender_peer_id);
    info!("[receiver] Destino local: {}/{}", announce.destination_path, announce.file_name);
    info!("[receiver] Enjambre recibido: {} peers", announce.swarm.len());
    for p in &announce.swarm {
        info!("[receiver]   peer {} @ {}:{}", p.peer_id, p.ip, p.tcp_port);
    }

    // 1. Registrar en el Tracker el enjambre recibido
    {
        let mut tracker = state.tracker.lock().await;
        let entry = tracker.get_or_create(&announce.transfer_id);
        entry.set_swarm(announce.swarm.clone());
        info!("Enjambre actualizado con {} peers.", announce.swarm.len());
        
        // El sender original ya tiene todos los chunks
        entry.add_peer_chunk(announce.sender_peer_id.clone(), 0); // At least one to start
        for i in 0..announce.total_chunks {
            entry.add_peer_chunk(announce.sender_peer_id.clone(), i);
        }
    }

    // 2. Preparar el archivo local
    let dest_path = Path::new(&announce.destination_path).join(&announce.file_name);
    info!("[receiver] Pre-asignando {} bytes en disco: {:?}", announce.file_size, dest_path);
    if let Err(e) = preallocate_file(&dest_path, announce.file_size) {
        error!("[receiver] Error al pre-asignar archivo: {} — verifica que el directorio exista y tengas permisos", e);
    }

    // 3. Registrar estado de la transferencia
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
        swarm: announce.swarm.clone(),
        sender_ip: announce.sender_ip.clone(),
        sender_peer_id: announce.sender_peer_id.clone(),
        bytes_transferred: 0,
        started_at: current_epoch(),
    };
    state.active_transfers.insert(announce.transfer_id.clone(), active.clone());
    let _ = app_handle.emit("transfer-incoming", &active);

    // 5. Lanzar hilo de descarga P2P activa ANTES de responder al sender.
    // Así el downloader siempre arranca aunque el sender haya cerrado la conexión.
    info!("[receiver] Lanzando motor de descarga P2P para '{}' ({} chunks)...", announce.file_name, announce.total_chunks);
    let state_clone = state.clone();
    let app_handle_clone = app_handle.clone();
    let announce_clone = announce.clone();
    tokio::spawn(async move {
        if let Err(e) = run_p2p_downloader(announce_clone, state_clone, app_handle_clone).await {
            error!("[receiver] Error fatal en motor P2P: {}", e);
        }
    });

    // 4. Intentar responder al sender (best-effort: el sender puede haber cerrado ya la conexión)
    let accepted = TransferAccepted {
        msg_type: TcpMsgType::TransferAccepted,
        transfer_id: announce.transfer_id.clone(),
        peer_id: state.peer_id.clone(),
    };
    match write_frame(&mut stream, &accepted, &[]).await {
        Ok(_) => info!("[receiver] TransferAccepted enviado al sender."),
        Err(e) => info!("[receiver] No se pudo enviar TransferAccepted (sender cerró la conexión): {} — esto es normal.", e),
    }

    Ok(())
}

/// Lógica de descarga BitTorrent: busca piezas en el enjambre y las descarga
async fn run_p2p_downloader(
    announce: TransferAnnounce,
    state: Arc<AppState>,
    app_handle: AppHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let transfer_id = &announce.transfer_id;
    let total_chunks = announce.total_chunks;

    let config = state.config.read().await;
    let max_concurrent = config.max_concurrent_chunks;
    let semaphore = Arc::new(Semaphore::new(max_concurrent));
    drop(config);

    info!("[p2p-downloader] Iniciando descarga '{}' | {} chunks | {} descargas simultáneas",
        announce.file_name, total_chunks, max_concurrent);

    let mut handles = Vec::new();

    for chunk_index in 0..total_chunks {
        // Si ya lo tenemos (por algún re-intento), saltar
        if let Some(t) = state.active_transfers.get(transfer_id) {
            if t.chunks_done[chunk_index as usize] { continue; }
        }

        let permit = semaphore.clone().acquire_owned().await?;
        let state_c = state.clone();
        let app_c = app_handle.clone();
        let ann_c = announce.clone();

        let h = tokio::spawn(async move {
            let _permit = permit;
            
            // 1. Buscar quién tiene la pieza
            let peer = {
                let tracker = state_c.tracker.lock().await;
                tracker.entries.get(&ann_c.transfer_id)
                    .and_then(|e| e.best_peer_for_chunk(chunk_index))
            };

            let Some(target_peer) = peer else {
                warn!("[p2p-downloader] Chunk {} no disponible en ningún peer todavía — se omite en esta ronda", chunk_index);
                return;
            };

            // 2. Descargar de ese peer
            debug!("[p2p-downloader] Chunk {}/{} → descargando de {}:{}", chunk_index + 1, ann_c.total_chunks, target_peer.ip, target_peer.tcp_port);
            if let Err(e) = download_chunk_from_peer(&target_peer, chunk_index, &ann_c, &state_c, &app_c).await {
                error!("[p2p-downloader] Error descargando chunk {} de {}: {}", chunk_index, target_peer.ip, e);
            }
        });
        handles.push(h);
    }

    for h in handles { let _ = h.await; }
    info!("[p2p-downloader] Proceso de descarga finalizado para transfer_id={}", transfer_id);

    Ok(())
}

async fn download_chunk_from_peer(
    peer: &SwarmPeer,
    chunk_index: u32,
    announce: &TransferAnnounce,
    state: &Arc<AppState>,
    app_handle: &AppHandle,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    debug!("Conectando a {}:{} para pieza {}...", peer.ip, peer.tcp_port, chunk_index);
    let stream = TcpStream::connect(format!("{}:{}", peer.ip, peer.tcp_port)).await?;
    let mut buf_stream = BufStream::new(stream);

    // 1. Pedir el chunk
    let req = ChunkRequest {
        msg_type: TcpMsgType::ChunkRequest,
        transfer_id: announce.transfer_id.clone(),
        chunk_index,
        requester_id: state.peer_id.clone(),
    };
    write_frame(&mut buf_stream, &req, &[]).await?;

    // 2. Recibir respuesta
    let (_res, data): (ChunkResponse, _) = read_typed_frame(&mut buf_stream).await?;

    // 3. Verificar Hash
    let expected_hash = &announce.chunk_hashes[chunk_index as usize];
    verify_chunk(&data, expected_hash)?;

    // 4. Escribir a disco
    let dest_path = Path::new(&announce.destination_path).join(&announce.file_name);
    write_chunk_at_offset(&dest_path, chunk_index, announce.chunk_size, &data)?;

    // 5. Actualizar estado local y capturar métricas para el evento
    let (is_complete, chunks_completed, bytes_transferred) = {
        if let Some(mut t) = state.active_transfers.get_mut(&announce.transfer_id) {
            t.chunks_done[chunk_index as usize] = true;
            t.bytes_transferred += data.len() as u64;
            let done = t.chunks_completed();
            let complete = done == t.total_chunks;
            (complete, done, t.bytes_transferred)
        } else {
            (false, 0, 0u64)
        }
    };

    // 6. Notificar al enjambre (HAVE)
    broadcast_have(announce.transfer_id.clone(), chunk_index, state).await;

    // 7. Emitir progreso con todos los campos que espera el frontend
    let _ = app_handle.emit("transfer-progress", serde_json::json!({
        "transfer_id": announce.transfer_id,
        "chunks_completed": chunks_completed,
        "total_chunks": announce.total_chunks,
        "bytes_transferred": bytes_transferred,
        "speed_bps": 0u64,
        "status": if is_complete { "Completed" } else { "InProgress" }
    }));

    if is_complete {
        info!("[p2p-downloader] ✓ TRANSFERENCIA COMPLETADA: '{}' — {} bytes recibidos",
            announce.file_name, bytes_transferred);
        let _ = app_handle.emit("transfer-complete", &announce.transfer_id);
    } else {
        debug!("[p2p-downloader] Chunk {}/{} completado ({} bytes total transferidos hasta ahora)",
            chunks_completed, announce.total_chunks, bytes_transferred);
    }

    Ok(())
}

/// Notifica a todos los vecinos del enjambre que tenemos una nueva pieza
async fn broadcast_have(transfer_id: String, chunk_index: u32, state: &Arc<AppState>) {
    let swarm = {
        let tracker = state.tracker.lock().await;
        tracker.entries.get(&transfer_id).map(|e| e.swarm.clone()).unwrap_or_default()
    };

    let msg = HaveChunk {
        msg_type: TcpMsgType::HaveChunk,
        transfer_id,
        peer_id: state.peer_id.clone(),
        chunk_index,
    };

    for peer in swarm {
        if peer.peer_id == state.peer_id { continue; }
        let ip = peer.ip.clone();
        let port = peer.tcp_port;
        let msg_c = msg.clone();
        tokio::spawn(async move {
            if let Ok(stream) = TcpStream::connect(format!("{}:{}", ip, port)).await {
                let mut buf = BufStream::new(stream);
                let _ = write_frame(&mut buf, &msg_c, &[]).await;
            }
        });
    }
}

/// Sirve un chunk a un peer que lo solicita
async fn handle_chunk_request(
    req: ChunkRequest,
    mut stream: BufStream<TcpStream>,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    debug!("[chunk-server] Solicitud: chunk {} para transfer {} de peer {}",
        req.chunk_index, req.transfer_id, req.requester_id);

    let transfer = state.active_transfers.get(&req.transfer_id);
    let Some(t) = transfer else {
        warn!("[chunk-server] Transfer ID '{}' desconocido — no puedo servir chunk {}", req.transfer_id, req.chunk_index);
        return Ok(());
    };

    // Solo servir si ya tenemos la pieza
    if !t.chunks_done[req.chunk_index as usize] {
        warn!("[chunk-server] Chunk {} solicitado pero no lo tenemos aún — peer debería reintentar", req.chunk_index);
        return Ok(());
    }

    let file_path = t.file_path.clone();
    let chunk_size = t.chunk_size;
    let chunk_hash = t.chunk_hashes[req.chunk_index as usize].clone();
    drop(t);

    // Leer del disco y enviar
    let hash_for_read = chunk_hash.clone();
    let data = tokio::task::spawn_blocking(move || {
        read_chunk(Path::new(&file_path), req.chunk_index, chunk_size, &hash_for_read)
    }).await??;

    let res = ChunkResponse {
        msg_type: TcpMsgType::ChunkResponse,
        transfer_id: req.transfer_id,
        chunk_index: req.chunk_index,
        sha1_hash: chunk_hash,
        chunk_size: data.len() as u32,
    };

    if let Err(e) = write_frame(&mut stream, &res, &data).await {
        error!("Error enviando pieza {} a un peer: {}", req.chunk_index, e);
    }
    Ok(())
}

/// Procesa una notificación "HAVE" de un vecino
async fn handle_have_chunk(
    have: HaveChunk,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut tracker = state.tracker.lock().await;
    let entry = tracker.get_or_create(&have.transfer_id);
    entry.add_peer_chunk(have.peer_id, have.chunk_index);
    Ok(())
}
