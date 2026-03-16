//! Receptor UDP (discovery) y TCP (transferencia).
//! - UDP: escucha broadcasts PeerAnnounce/PeerBye en puerto 47832
//! - TCP: escucha conexiones entrantes de transferencia en puerto 47833

use std::net::{SocketAddrV4, Ipv4Addr};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::broadcast;
use tokio::time::{interval, Duration};
use tauri::{AppHandle, Emitter};
use tracing::{debug, error, info, warn};

use crate::network::discovery::{TCP_PORT, UDP_PORT};
use crate::protocol::messages::{PeerAnnounce, PeerBye, UdpMsgType};
use crate::state::{AppState, PeerEntry, PeerKind};
use crate::transfer::receiver;

fn current_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Tarea que escucha mensajes UDP de discovery
pub async fn run_udp_listener(
    state: Arc<AppState>,
    app_handle: AppHandle,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    // Crear socket UDP con SO_REUSEADDR para que múltiples instancias puedan escuchar
    let std_socket = {
        let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
            .expect("No se pudo crear socket UDP");
        s.set_reuse_address(true).ok();
        s.set_broadcast(true).ok();
        s.set_nonblocking(true).ok();
        s.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, UDP_PORT).into())
            .expect("No se pudo bindear socket UDP");
        s.into()
    };

    let socket = UdpSocket::from_std(std_socket).expect("No se pudo convertir socket UDP");
    let mut buf = vec![0u8; 512];

    info!("Listener UDP iniciado en puerto {}", UDP_PORT);

    // También lanzar limpieza de peers stale cada 30s
    let state_cleanup = state.clone();
    let app_handle_cleanup = app_handle.clone();
    tokio::spawn(async move {
        run_peer_cleanup(state_cleanup, app_handle_cleanup).await;
    });

    loop {
        tokio::select! {
            result = socket.recv_from(&mut buf) => {
                match result {
                    Ok((n, addr)) => {
                        let data = &buf[..n];
                        handle_udp_message(data, addr.ip().to_string(), &state, &app_handle).await;
                    }
                    Err(e) => {
                        error!("Error recibiendo UDP: {}", e);
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!("Listener UDP: shutdown recibido");
                break;
            }
        }
    }
}

async fn handle_udp_message(
    data: &[u8],
    from_ip: String,
    state: &Arc<AppState>,
    app_handle: &AppHandle,
) {
    // Detectar tipo de mensaje
    #[derive(serde::Deserialize)]
    struct MsgTypeOnly { msg_type: UdpMsgType }

    let Ok(envelope) = serde_json::from_slice::<MsgTypeOnly>(data) else {
        debug!("UDP: mensaje no reconocido desde {}", from_ip);
        return;
    };

    match envelope.msg_type {
        UdpMsgType::PeerAnnounce => {
            if let Ok(announce) = serde_json::from_slice::<PeerAnnounce>(data) {
                // Ignorar mensajes propios
                if announce.peer_id == state.peer_id {
                    return;
                }
                let entry = PeerEntry {
                    peer_id: announce.peer_id.clone(),
                    hostname: announce.hostname.clone(),
                    ip: announce.ip.clone(),
                    tcp_port: announce.tcp_port,
                    kind: PeerKind::App,
                    last_seen: current_epoch(),
                    online: true,
                    app_version: Some(announce.app_version.clone()),
                };
                state.peers.insert(announce.peer_id.clone(), entry.clone());
                if let Err(e) = app_handle.emit("peer-updated", &entry) {
                    error!("Error emitiendo peer-updated: {}", e);
                }
                debug!("Peer actualizado: {} ({})", announce.hostname, announce.ip);
            }
        }
        UdpMsgType::PeerBye => {
            if let Ok(bye) = serde_json::from_slice::<PeerBye>(data) {
                if bye.peer_id == state.peer_id {
                    return;
                }
                state.peers.remove(&bye.peer_id);
                if let Err(e) = app_handle.emit("peer-removed", &bye.peer_id) {
                    error!("Error emitiendo peer-removed: {}", e);
                }
                info!("Peer desconectado: {}", bye.ip);
            }
        }
        UdpMsgType::ChunkAvailable => {
            // El tracker actualiza quién tiene qué chunks
            // (manejado en transfer/tracker.rs vía evento)
            debug!("ChunkAvailable recibido de {}", from_ip);
        }
    }
}

/// Limpia peers que no han anunciado presencia en más de 30 segundos
async fn run_peer_cleanup(state: Arc<AppState>, app_handle: AppHandle) {
    let mut ticker = interval(Duration::from_secs(10));
    loop {
        ticker.tick().await;
        let now = current_epoch();
        let stale_threshold = 30u64;

        let stale_ids: Vec<String> = state
            .peers
            .iter()
            .filter(|entry| {
                entry.kind == PeerKind::App && now.saturating_sub(entry.last_seen) > stale_threshold
            })
            .map(|entry| entry.peer_id.clone())
            .collect();

        for peer_id in stale_ids {
            if let Some((_, mut entry)) = state.peers.remove(&peer_id) {
                entry.online = false;
                warn!("Peer stale removido: {} ({})", entry.hostname, entry.ip);
                if let Err(e) = app_handle.emit("peer-removed", &peer_id) {
                    error!("Error emitiendo peer-removed: {}", e);
                }
            }
        }
    }
}

/// Tarea que escucha conexiones TCP entrantes de transferencia
pub async fn run_tcp_listener(
    state: Arc<AppState>,
    app_handle: AppHandle,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let listener = match TcpListener::bind(format!("0.0.0.0:{}", TCP_PORT)).await {
        Ok(l) => l,
        Err(e) => {
            error!("No se pudo iniciar TCP listener en puerto {}: {}", TCP_PORT, e);
            return;
        }
    };

    info!("Listener TCP iniciado en puerto {}", TCP_PORT);

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, addr)) => {
                        info!("Conexión TCP entrante desde {}", addr);
                        let state_clone = state.clone();
                        let app_handle_clone = app_handle.clone();
                        tokio::spawn(async move {
                            if let Err(e) = receiver::handle_incoming_connection(
                                stream,
                                state_clone,
                                app_handle_clone,
                            ).await {
                                error!("Error manejando conexión TCP: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("Error aceptando conexión TCP: {}", e);
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!("Listener TCP: shutdown recibido");
                break;
            }
        }
    }
}
