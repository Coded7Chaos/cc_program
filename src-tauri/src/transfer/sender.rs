//! Sender: inicia transferencias y actúa como la semilla (seed) inicial del enjambre.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::net::TcpStream;
use tokio::io::BufStream;
use tracing::{info, error, warn};

/// Timeout para conectar al receptor al enviar el anuncio.
/// 5 s es generoso para LAN Gigabit; evita que el sender quede colgado si la IP es incorrecta.
const ANNOUNCE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
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
    app_handle: AppHandle,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    info!("╔══════════════════════════════════════╗");
    info!("║   INICIANDO PROCESO DE ENVÍO P2P     ║");
    info!("╚══════════════════════════════════════╝");
    info!("[sender] Peers destino: {:?}", target_peer_ids);

    let path = Path::new(&file_path);
    if !path.exists() {
        error!("[sender] Error crítico: El archivo no existe en '{}'", file_path);
        return Err("Archivo no encontrado".into());
    }

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("archivo")
        .to_string();
    let file_size = std::fs::metadata(path)?.len();
    let total_chunks = chunk_count(file_size, CHUNK_SIZE);

    info!("[sender] Archivo: '{}' | Tamaño: {} bytes | Chunks: {} x {} MB",
        file_name, file_size, total_chunks, CHUNK_SIZE / (1024 * 1024));

    // 1. Calcular hashes
    let path_clone = path.to_path_buf();
    let app_clone = app_handle.clone();
    info!("[sender] Calculando hashes SHA1 de {} chunks (emitiendo hash-progress)...", total_chunks);
    let t_hash = std::time::Instant::now();
    let chunk_hashes = tokio::task::spawn_blocking(move || {
        build_chunk_map(&path_clone, CHUNK_SIZE, Some(app_clone))
    }).await??;
    info!("[sender] Hashes calculados en {}ms.", t_hash.elapsed().as_millis());

    let transfer_id = Uuid::new_v4().to_string();
    info!("[sender] Transfer ID: {}", transfer_id);

    // 2. Construir la lista del Enjambre (Swarm)
    let mut swarm = Vec::new();
    let my_port = *state.tcp_port.read().await;
    // Añadirse a sí mismo como el primer peer (la semilla)
    swarm.push(SwarmPeer {
        peer_id: state.peer_id.clone(),
        ip: state.local_ip.clone(),
        tcp_port: my_port,
    });
    info!("[sender] Semilla (yo): {}:{}", state.local_ip, my_port);

    for peer_id in &target_peer_ids {
        if let Some(peer) = state.peers.get(peer_id) {
            info!("[sender] Añadiendo al enjambre: {} ({}:{})", peer.hostname, peer.ip, peer.tcp_port);
            swarm.push(SwarmPeer {
                peer_id: peer_id.clone(),
                ip: peer.ip.clone(),
                tcp_port: peer.tcp_port,
            });
        } else {
            warn!("[sender] Peer ID '{}' no encontrado en el mapa de peers — se omite", peer_id);
        }
    }
    info!("[sender] Enjambre total: {} participantes.", swarm.len());

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
        bytes_transferred: file_size, // El sender ya transfirió todo (es la semilla)
        started_at: current_epoch(),
    };
    // Notificar a la UI que el sender inició una transferencia (aparece en el monitor)
    info!("[sender] Emitiendo transfer-incoming para la UI del sender...");
    let _ = app_handle.emit("transfer-incoming", &active);
    state.active_transfers.insert(transfer_id.clone(), active);

    // 5. Notificar a todos los peers para que se unan al enjambre.
    //    Se espera la respuesta de cada announce (con timeout) para detectar fallos reales.
    info!("[sender] Enviando TransferAnnounce a {} receptores...", target_peer_ids.len());
    info!("[sender] Mi IP anunciada al enjambre: {} (puerto {})", state.local_ip, my_port);

    let mut announce_handles = Vec::new();
    for peer_id in target_peer_ids {
        let peer = match state.peers.get(&peer_id) {
            Some(p) => p.clone(),
            None => {
                warn!("[sender] Peer '{}' desapareció del mapa justo antes del anuncio", peer_id);
                continue;
            }
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

        let ip = peer.ip.clone();
        let port = peer.tcp_port;
        let tid = transfer_id.clone();
        let handle = tokio::spawn(async move {
            info!("[sender → {}:{}] Conectando para enviar TransferAnnounce...", ip, port);
            let connect_result = tokio::time::timeout(
                ANNOUNCE_CONNECT_TIMEOUT,
                TcpStream::connect(format!("{}:{}", ip, port)),
            ).await;

            match connect_result {
                Ok(Ok(stream)) => {
                    let mut buf = BufStream::new(stream);
                    match write_frame(&mut buf, &announce, &[]).await {
                        Ok(_) => {
                            info!("[sender → {}] ✓ TransferAnnounce entregado. Transfer: {}", ip, tid);
                            true
                        }
                        Err(e) => {
                            error!("[sender → {}] ✗ Error al escribir TransferAnnounce: {}", ip, e);
                            false
                        }
                    }
                }
                Ok(Err(e)) => {
                    error!("[sender → {}:{}] ✗ Conexión rechazada: {} — ¿está la app corriendo? ¿Firewall?", ip, port, e);
                    false
                }
                Err(_) => {
                    error!("[sender → {}:{}] ✗ Timeout ({}s) al conectar — ¿IP incorrecta? ¿Firewall bloqueando?",
                        ip, port, ANNOUNCE_CONNECT_TIMEOUT.as_secs());
                    false
                }
            }
        });
        announce_handles.push((peer.ip.clone(), handle));
    }

    // Esperar a que todos los announces terminen para tener el conteo real
    let mut delivered = 0usize;
    let mut failed = 0usize;
    for (ip, handle) in announce_handles {
        match handle.await {
            Ok(true)  => delivered += 1,
            Ok(false) => failed += 1,
            Err(e)    => { error!("[sender] Tarea de announce panicked para {}: {}", ip, e); failed += 1; }
        }
    }

    info!("[sender] Announces: {} entregados, {} fallidos.", delivered, failed);
    if failed > 0 {
        warn!("[sender] {} receptores no recibieron el anuncio. Causas posibles:", failed);
        warn!("  1. La app no está corriendo en esas máquinas");
        warn!("  2. Windows Firewall bloqueando el puerto en el receptor");
        warn!("  3. IP del receptor cambió desde el último escaneo");
    }

    // El sender ya tiene todos los chunks: marcar como completado
    if let Some(mut t) = state.active_transfers.get_mut(&transfer_id) {
        t.status = TransferStatus::Completed;
    }
    let _ = app_handle.emit("transfer-complete", &transfer_id);
    info!("[sender] Listo. Sirviendo chunks bajo demanda ({} receptores activos).", delivered);

    Ok(transfer_id)
}
