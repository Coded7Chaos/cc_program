//! Escanea la subnet local usando la tabla ARP y peticiones ARP (via ping) para encontrar dispositivos.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::sync::broadcast;
use tauri::{AppHandle, Emitter};
use tracing::{debug, error, info};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

use crate::state::{AppState, PeerEntry, PeerKind};

const PING_TIMEOUT_MS: u64 = 500;
const PROBE_TIMEOUT_MS: u64 = 200; // Sondeo rápido para red local
const MAX_CONCURRENT_PINGS: usize = 64;

// Escanear un rango de puertos para permitir múltiples instancias en la misma PC
const START_PORT: u16 = 47833;
const PORT_RANGE: u16 = 10; 

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
            let octets = ip.octets();
            if octets[0] == 169 && octets[1] == 254 {
                continue;
            }
            if octets[0] == 127 {
                continue;
            }
            let base = Ipv4Addr::new(octets[0], octets[1], octets[2], 0);
            return Some((base, 254));
        }
    }
    None
}

/// Obtiene la tabla ARP del sistema
async fn get_arp_table() -> Vec<(String, String)> {
    let output = if cfg!(target_os = "windows") {
        let mut cmd = Command::new("arp");
        cmd.arg("-a");
        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.output().await
    } else {
        Command::new("arp").arg("-an").output().await
    };

    let mut entries = Vec::new();
    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if cfg!(target_os = "macos") {
                if let (Some(start), Some(end)) = (line.find('('), line.find(')')) {
                    let ip = &line[start + 1..end];
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(at_idx) = parts.iter().position(|&r| r == "at") {
                        if let Some(mac) = parts.get(at_idx + 1) {
                            if *mac != "(incomplete)" {
                                entries.push((ip.to_string(), mac.to_string()));
                            }
                        }
                    }
                }
            } else if cfg!(target_os = "windows") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let ip = parts[0];
                    let mac = parts[1];
                    if ip.contains('.') && mac.contains('-') {
                        entries.push((ip.to_string(), mac.to_string()));
                    }
                }
            } else {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    let ip = parts[0];
                    let mac = parts[2];
                    if ip.contains('.') && mac.contains(':') {
                        entries.push((ip.to_string(), mac.to_string()));
                    }
                }
            }
        }
    }
    entries
}

/// Dispara una petición ARP haciendo un ping rápido
async fn trigger_arp(ip: Ipv4Addr) {
    let _ = if cfg!(target_os = "windows") {
        let mut cmd = Command::new("ping");
        cmd.args(["-n", "1", "-w", &PING_TIMEOUT_MS.to_string(), &ip.to_string()]);
        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.output().await
    } else {
        Command::new("ping")
            .args(["-c", "1", "-t", "1", &ip.to_string()])
            .output()
            .await
    };
}

/// Verifica si un peer tiene la app respondiendo en el rango de puertos TCP.
/// Retorna el puerto que respondió, si existe.
async fn find_app_port(ip: &str) -> Option<u16> {
    // Primero probar el puerto por defecto (el más probable) de forma individual para ahorrar recursos
    if probe_single_port(ip, START_PORT).await {
        return Some(START_PORT);
    }

    // Si no es el default, probar el resto en paralelo
    let mut tasks = Vec::new();
    for port in (START_PORT + 1)..=(START_PORT + PORT_RANGE) {
        let ip_str = ip.to_string();
        tasks.push(tokio::spawn(async move {
            if probe_single_port(&ip_str, port).await {
                Some(port)
            } else {
                None
            }
        }));
    }

    for task in tasks {
        if let Ok(Some(port)) = task.await {
            return Some(port);
        }
    }

    None
}

async fn probe_single_port(ip: &str, port: u16) -> bool {
    let Ok(addr) = format!("{}:{}", ip, port).parse::<SocketAddr>() else { return false; };
    tokio::time::timeout(
        Duration::from_millis(PROBE_TIMEOUT_MS),
        TcpStream::connect(addr),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}

/// Realiza escaneo de la subnet usando ARP
pub async fn run_subnet_scan(
    state: Arc<AppState>,
    app_handle: AppHandle,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let Some((base_ip, count)) = get_local_subnet() else {
        error!("No se pudo detectar subnet local");
        return;
    };

    info!("Iniciando escaneo ARP de la subnet con sondeo multi-puerto...");

    // 1. Consultar caché ARP actual
    let initial_entries = get_arp_table().await;
    for (ip, mac) in initial_entries {
        process_discovered_peer(&ip, &mac, &state, &app_handle).await;
    }

    // 2. Disparar peticiones ARP barriendo la subnet (pings)
    let base_octets = base_ip.octets();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_PINGS));
    let mut handles = Vec::new();

    for i in 1..=count {
        if shutdown_rx.try_recv().is_ok() {
            break;
        }

        let target_ip = Ipv4Addr::new(base_octets[0], base_octets[1], base_octets[2], i as u8);
        if target_ip.to_string() == state.local_ip {
            continue;
        }

        let sem = semaphore.clone();
        let handle = tokio::spawn(async move {
            let _permit = sem.acquire().await.ok()?;
            trigger_arp(target_ip).await;
            Some(())
        });
        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.await;
    }

    // 3. Consultar caché ARP de nuevo tras los pings
    let final_entries = get_arp_table().await;
    let mut found_count = 0;
    for (ip, mac) in final_entries {
        if process_discovered_peer(&ip, &mac, &state, &app_handle).await {
            found_count += 1;
        }
    }

    let _ = app_handle.emit("scan-progress", serde_json::json!({
        "scanned": count,
        "total": count,
        "found": found_count
    }));

    info!("Escaneo ARP completado. {} dispositivos encontrados.", found_count);
}

async fn process_discovered_peer(ip: &str, mac: &str, state: &Arc<AppState>, app_handle: &AppHandle) -> bool {
    if ip.starts_with("224.") || ip.starts_with("239.") || ip == "255.255.255.255" {
        return false;
    }

    // Probar si tiene la app en algún puerto del rango
    let app_port = find_app_port(ip).await;
    let has_app = app_port.is_some();
    let kind = if has_app { PeerKind::App } else { PeerKind::NonApp };
    let tcp_port = app_port.unwrap_or(0);

    let mut updated = false;
    let peer_id = if has_app { format!("{}:{}", ip, tcp_port) } else { ip.to_string() };

    // Intentar actualizar por IP (DashMap no permite iteración mutable fácil así que usamos remove/insert si el ID cambia)
    // Para simplificar, usaremos IP + Puerto como ID único para App peers.
    
    state.peers.alter(&peer_id, |_, mut entry| {
        entry.mac_address = Some(mac.to_string());
        entry.last_seen = current_epoch();
        entry.online = true;
        entry.kind = kind.clone();
        entry.tcp_port = tcp_port;
        updated = true;
        entry
    });

    if !updated {
        let peer_entry = PeerEntry {
            peer_id: peer_id.clone(),
            hostname: format!("Device-{}", &mac.replace(":", "").get(0..6).unwrap_or("unknown")),
            ip: ip.to_string(),
            mac_address: Some(mac.to_string()),
            tcp_port,
            kind,
            last_seen: current_epoch(),
            online: true,
            app_version: None,
        };
        state.peers.insert(peer_id, peer_entry.clone());
        let _ = app_handle.emit("peer-updated", &peer_entry);
        return true;
    }
    
    false
}
