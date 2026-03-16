//! Broadcaster UDP periódico para anunciar presencia en la red LAN.
//! Envía PeerAnnounce cada 5 segundos y PeerBye al salir.

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tokio::time::{interval, Duration};
use tracing::{error, info};

use crate::protocol::messages::{PeerAnnounce, PeerBye, UdpMsgType};
use crate::state::AppState;

pub const UDP_PORT: u16 = 47832;
pub const TCP_PORT: u16 = 47833;
pub const APP_VERSION: &str = "0.1.0";
/// Intervalo de anuncio en segundos
const ANNOUNCE_INTERVAL_SECS: u64 = 5;

fn current_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Crea un socket UDP con SO_BROADCAST habilitado
fn create_broadcast_socket() -> std::io::Result<UdpSocket> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_broadcast(true)?;
    Ok(socket)
}

/// Envía un PeerAnnounce a la dirección de broadcast de la red
pub fn send_announce(state: &AppState) -> Result<(), Box<dyn std::error::Error>> {
    let socket = create_broadcast_socket()?;
    let announce = PeerAnnounce {
        msg_type: UdpMsgType::PeerAnnounce,
        peer_id: state.peer_id.clone(),
        hostname: state.hostname.clone(),
        ip: state.local_ip.clone(),
        tcp_port: TCP_PORT,
        app_version: APP_VERSION.to_string(),
        timestamp: current_epoch(),
    };
    let data = serde_json::to_vec(&announce)?;
    let broadcast_addr = SocketAddrV4::new(Ipv4Addr::BROADCAST, UDP_PORT);
    socket.send_to(&data, broadcast_addr)?;
    Ok(())
}

/// Envía un PeerBye indicando que este peer sale de la red
pub fn send_bye(state: &AppState) -> Result<(), Box<dyn std::error::Error>> {
    let socket = create_broadcast_socket()?;
    let bye = PeerBye {
        msg_type: UdpMsgType::PeerBye,
        peer_id: state.peer_id.clone(),
        ip: state.local_ip.clone(),
    };
    let data = serde_json::to_vec(&bye)?;
    let broadcast_addr = SocketAddrV4::new(Ipv4Addr::BROADCAST, UDP_PORT);
    socket.send_to(&data, broadcast_addr)?;
    Ok(())
}

/// Tarea async que envía PeerAnnounce cada ANNOUNCE_INTERVAL_SECS segundos
/// hasta que se recibe señal de shutdown.
pub async fn run_broadcaster(state: Arc<AppState>, mut shutdown_rx: broadcast::Receiver<()>) {
    let mut ticker = interval(Duration::from_secs(ANNOUNCE_INTERVAL_SECS));

    info!("Broadcaster UDP iniciado (peer_id={})", state.peer_id);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Err(e) = send_announce(&state) {
                    error!("Error enviando PeerAnnounce: {}", e);
                }
            }
            _ = shutdown_rx.recv() => {
                info!("Broadcaster UDP: shutdown recibido");
                // Enviar PeerBye antes de salir
                if let Err(e) = send_bye(&state) {
                    error!("Error enviando PeerBye: {}", e);
                }
                break;
            }
        }
    }
}
