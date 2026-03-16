//! Escanea la subnet local buscando PCs sin la app via TCP:445 (SMB).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tauri::{AppHandle, Emitter};
use tracing::{debug, error, info};

use crate::state::{AppState, PeerEntry, PeerKind};

const SMB_PORT: u16 = 445;
const SCAN_TIMEOUT_MS: u64 = 300;
const MAX_CONCURRENT_SCANS: usize = 50;

fn current_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Detecta la subnet local y retorna el rango de IPs a escanear
fn get_local_subnet() -> Option<(Ipv4Addr, u32)> {
    let addrs = if_addrs::get_if_addrs().ok()?;
    for iface in addrs {
        if iface.is_loopback() {
            continue;
        }
        if let IpAddr::V4(ip) = iface.ip() {
            // Ignorar IPs de VM (169.254.x.x) y loopback
            let octets = ip.octets();
            if octets[0] == 169 && octets[1] == 254 {
                continue;
            }
            if octets[0] == 127 {
                continue;
            }
            // Calcular base de /24
            let base = Ipv4Addr::new(octets[0], octets[1], octets[2], 0);
            return Some((base, 254)); // .1 a .254
        }
    }
    None
}

/// Testa si una IP tiene puerto SMB:445 abierto (indica PC Windows activa)
async fn probe_smb(ip: Ipv4Addr) -> bool {
    let addr = SocketAddr::new(IpAddr::V4(ip), SMB_PORT);
    tokio::time::timeout(
        Duration::from_millis(SCAN_TIMEOUT_MS),
        TcpStream::connect(addr),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}

/// Realiza escaneo de la subnet, inserta PCs sin app como NonApp peers,
/// emite eventos scan-progress
pub async fn run_subnet_scan(
    state: Arc<AppState>,
    app_handle: AppHandle,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let Some((base_ip, count)) = get_local_subnet() else {
        error!("No se pudo detectar subnet local");
        return;
    };

    let base_octets = base_ip.octets();
    info!(
        "Iniciando escaneo de subnet {}.{}.{}.0/24",
        base_octets[0], base_octets[1], base_octets[2]
    );

    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_SCANS));
    let mut handles = Vec::new();
    let mut scanned = 0u32;
    let total = count;
    let mut found = 0u32;

    for i in 1..=count {
        // Verificar shutdown
        if shutdown_rx.try_recv().is_ok() {
            break;
        }

        let target_ip = Ipv4Addr::new(base_octets[0], base_octets[1], base_octets[2], i as u8);

        // Saltar IP propia
        if target_ip.to_string() == state.local_ip {
            scanned += 1;
            continue;
        }

        // Saltar si ya está como App peer
        let already_app = state.peers.iter().any(|e| e.ip == target_ip.to_string() && e.kind == PeerKind::App);
        if already_app {
            scanned += 1;
            continue;
        }

        let sem = semaphore.clone();
        let state_clone = state.clone();
        let app_handle_clone = app_handle.clone();

        let handle = tokio::spawn(async move {
            let _permit = sem.acquire().await.ok()?;
            if probe_smb(target_ip).await {
                let peer_entry = PeerEntry {
                    peer_id: target_ip.to_string(), // usar IP como ID para NonApp
                    hostname: target_ip.to_string(),
                    ip: target_ip.to_string(),
                    tcp_port: 0,
                    kind: PeerKind::NonApp,
                    last_seen: current_epoch(),
                    online: true,
                    app_version: None,
                };
                state_clone.peers.insert(target_ip.to_string(), peer_entry.clone());
                if let Err(e) = app_handle_clone.emit("peer-updated", &peer_entry) {
                    error!("Error emitiendo peer-updated (NonApp): {}", e);
                }
                debug!("NonApp peer encontrado: {}", target_ip);
                return Some(());
            }
            None
        });

        handles.push(handle);
        scanned += 1;

        // Emitir progreso cada 10 IPs
        if scanned % 10 == 0 {
            let _ = app_handle.emit("scan-progress", serde_json::json!({
                "scanned": scanned,
                "total": total,
                "found": found
            }));
        }
    }

    // Esperar todos los probes
    for handle in handles {
        if let Ok(Some(())) = handle.await {
            found += 1;
        }
    }

    // Progreso final
    let _ = app_handle.emit("scan-progress", serde_json::json!({
        "scanned": total,
        "total": total,
        "found": found
    }));

    info!("Escaneo completado: {}/{} IPs, {} PCs encontradas", scanned, total, found);
}
