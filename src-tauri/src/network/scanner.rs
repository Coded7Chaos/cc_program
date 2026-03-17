//! Escanea la subnet local usando la tabla ARP y peticiones ARP (via ping) para encontrar
//! dispositivos. La detección de que una máquina tiene la app activa se hace mediante una
//! conexión TCP al puerto del listener: si conecta, la máquina está encendida, en la red
//! y con el proceso listener corriendo.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::sync::broadcast;
use tauri::{AppHandle, Emitter};
use tracing::{error, info};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

use crate::state::{AppState, PeerEntry, PeerKind};

const PING_TIMEOUT_MS: u64 = 500;
/// Timeout de sondeo TCP. En red Gigabit local 150 ms es más que suficiente.
const PROBE_TIMEOUT_MS: u64 = 150;
const MAX_CONCURRENT_PINGS: usize = 64;
/// Sondeos TCP concurrentes — evita saturar el stack de red en máquinas lentas.
const MAX_CONCURRENT_PROBES: usize = 32;

// Rango de puertos de la app (permite múltiples instancias en la misma PC)
const START_PORT: u16 = 47833;
const PORT_RANGE: u16 = 10;

fn current_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Detecta la subnet local y retorna la IP base y la cantidad de hosts (/24)
fn get_local_subnet() -> Option<(Ipv4Addr, u32)> {
    let addrs = if_addrs::get_if_addrs().ok()?;
    for iface in addrs {
        if iface.is_loopback() {
            continue;
        }
        if let IpAddr::V4(ip) = iface.ip() {
            let octets = ip.octets();
            if octets[0] == 169 && octets[1] == 254 {
                continue; // APIPA
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

/// Obtiene la tabla ARP del sistema operativo
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
                // Linux
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

/// Dispara una petición ARP mediante un ping de un solo paquete.
/// No importa si la máquina bloquea ICMP: el OS ya envía la solicitud ARP
/// para resolver la MAC antes de intentar el ping.
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

/// Verifica si una IP tiene el listener de la app activo en el rango de puertos.
/// Sondea todos los puertos del rango SIMULTÁNEAMENTE y retorna el primero que acepta
/// la conexión TCP. Si ninguno responde dentro de PROBE_TIMEOUT_MS, retorna None.
///
/// Una conexión TCP exitosa garantiza que la máquina está:
///   (1) encendida, (2) conectada a la red, (3) con el proceso listener activo.
async fn find_app_port(ip: &str) -> Option<u16> {
    let mut tasks = Vec::new();
    for port in START_PORT..=(START_PORT + PORT_RANGE) {
        let ip_str = ip.to_string();
        tasks.push(tokio::spawn(async move {
            if probe_single_port(&ip_str, port).await {
                Some(port)
            } else {
                None
            }
        }));
    }

    // Todos los tasks corren en paralelo. Recorremos en orden de puerto ascendente
    // para preferir el puerto base cuando hay varias instancias en la misma PC.
    // El tiempo total siempre es ≈ PROBE_TIMEOUT_MS (no multiplicativo).
    for task in tasks {
        if let Ok(Some(port)) = task.await {
            return Some(port);
        }
    }
    None
}

async fn probe_single_port(ip: &str, port: u16) -> bool {
    let Ok(addr) = format!("{}:{}", ip, port).parse::<SocketAddr>() else {
        return false;
    };
    tokio::time::timeout(
        Duration::from_millis(PROBE_TIMEOUT_MS),
        TcpStream::connect(addr),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}

/// Escaneo de subnet en tres fases solapadas para minimizar el tiempo total:
///
///  Fase A (paralela con B): TCP-probe de IPs ya en caché ARP → actualiza peers existentes
///  Fase B (paralela con A): Pings a toda la /24 → popula ARP con nuevas IPs
///  Fase C (tras A+B):       TCP-probe de las IPs recién descubiertas por los pings
pub async fn run_subnet_scan(
    state: Arc<AppState>,
    app_handle: AppHandle,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let Some((base_ip, count)) = get_local_subnet() else {
        error!("No se pudo detectar subnet local");
        return;
    };

    info!("Iniciando escaneo ARP de la subnet ({} hosts)...", count);

    // ── Leer caché ARP inicial (sin I/O de red, casi instantáneo) ──────────────
    let initial_entries = get_arp_table().await;
    let initial_ips: std::collections::HashSet<String> =
        initial_entries.iter().map(|(ip, _)| ip.clone()).collect();

    // ── Fase B: lanzar pings a toda la subnet (poblar ARP con nuevas IPs) ──────
    let base_octets = base_ip.octets();
    let ping_sem = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_PINGS));
    let mut ping_handles = Vec::new();

    for i in 1..=count {
        if shutdown_rx.try_recv().is_ok() {
            break;
        }
        let target_ip = Ipv4Addr::new(base_octets[0], base_octets[1], base_octets[2], i as u8);
        if target_ip.to_string() == state.local_ip {
            continue;
        }
        let sem = ping_sem.clone();
        ping_handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.ok()?;
            trigger_arp(target_ip).await;
            Some(())
        }));
    }

    // ── Fase A: TCP-probe de las IPs ya en ARP, en paralelo con los pings ──────
    let probe_sem = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_PROBES));
    let mut probe_handles: Vec<tokio::task::JoinHandle<bool>> = Vec::new();

    for (ip, mac) in initial_entries.iter() {
        if is_ignored_ip(ip) {
            continue;
        }
        let ip = ip.clone();
        let mac = mac.clone();
        let state_c = state.clone();
        let app_handle_c = app_handle.clone();
        let sem = probe_sem.clone();
        probe_handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.ok();
            process_discovered_peer(&ip, &mac, &state_c, &app_handle_c).await
        }));
    }

    // Esperar que terminen AMBAS fases antes de leer ARP de nuevo
    for h in ping_handles {
        let _ = h.await;
    }
    for h in probe_handles {
        let _ = h.await;
    }

    // ── Fase C: leer ARP final, sondear solo IPs nuevas (no cubiertas en fase A) ─
    let final_entries = get_arp_table().await;

    // Unión de IPs vistas en ambas lecturas para el marcado de offline al final
    let mut seen_ips = initial_ips.clone();
    for (ip, _) in &final_entries {
        seen_ips.insert(ip.clone());
    }

    let mut new_probe_handles: Vec<tokio::task::JoinHandle<bool>> = Vec::new();
    for (ip, mac) in &final_entries {
        if is_ignored_ip(ip) || initial_ips.contains(ip) {
            continue; // ya procesada en fase A
        }
        let ip = ip.clone();
        let mac = mac.clone();
        let state_c = state.clone();
        let app_handle_c = app_handle.clone();
        let sem = probe_sem.clone();
        new_probe_handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.ok();
            process_discovered_peer(&ip, &mac, &state_c, &app_handle_c).await
        }));
    }

    let mut found_count = 0usize;
    for h in new_probe_handles {
        if let Ok(true) = h.await {
            found_count += 1;
        }
    }

    // ── Marcar offline los peers que no aparecieron en ninguna lectura ARP ──────
    // Esto detecta máquinas que se apagaron o desconectaron desde el último escaneo.
    for mut entry in state.peers.iter_mut() {
        if !seen_ips.contains(&entry.ip) && entry.online {
            info!(
                "[scanner] Peer {} ({}) ausente del ARP — marcando offline",
                entry.hostname, entry.ip
            );
            entry.online = false;
            let _ = app_handle.emit("peer-updated", entry.value());
        }
    }

    let _ = app_handle.emit(
        "scan-progress",
        serde_json::json!({
            "scanned": count,
            "total": count,
            "found": found_count
        }),
    );

    info!(
        "[scanner] Escaneo completado. {} nuevos dispositivos con app detectados.",
        found_count
    );
}

/// Retorna true para IPs de multicast/broadcast que no deben procesarse.
fn is_ignored_ip(ip: &str) -> bool {
    ip.starts_with("224.")
        || ip.starts_with("239.")
        || ip == "255.255.255.255"
}

/// Procesa un dispositivo descubierto en ARP:
///  - Hace TCP probe para saber si tiene la app activa (y en qué puerto)
///  - Actualiza o crea la entrada en el mapa de peers
///  - Emite evento peer-updated o peer-removed según corresponda
///
/// Retorna true solo si es un peer NUEVO (no existía antes en el mapa).
async fn process_discovered_peer(
    ip: &str,
    mac: &str,
    state: &Arc<AppState>,
    app_handle: &AppHandle,
) -> bool {
    if is_ignored_ip(ip) {
        return false;
    }

    // TCP probe: verifica encendido + conectividad + listener activo, todo en uno
    let app_port = find_app_port(ip).await;
    let has_app = app_port.is_some();
    let kind = if has_app { PeerKind::App } else { PeerKind::NonApp };
    let tcp_port = app_port.unwrap_or(0);
    let peer_id = if has_app {
        format!("{}:{}", ip, tcp_port)
    } else {
        ip.to_string()
    };

    // Eliminar entradas obsoletas para esta IP con peer_id diferente.
    // Caso típico: la app se instaló (clave "ip" → "ip:puerto") o se desinstalió (al revés),
    // o la instancia cambió de puerto. Sin esto quedan "fantasmas" permanentes.
    let stale_keys: Vec<String> = state
        .peers
        .iter()
        .filter(|e| e.ip == ip && e.peer_id != peer_id)
        .map(|e| e.peer_id.clone())
        .collect();
    for stale_key in stale_keys {
        info!(
            "[scanner] Eliminando entrada obsoleta '{}' (IP '{}', nuevo peer_id '{}')",
            stale_key, ip, peer_id
        );
        state.peers.remove(&stale_key);
        let _ = app_handle.emit("peer-removed", &stale_key);
    }

    // Actualizar entrada existente
    let mut found_existing = false;
    state.peers.alter(&peer_id, |_, mut entry| {
        entry.mac_address = Some(mac.to_string());
        entry.last_seen = current_epoch();
        entry.online = true;
        entry.kind = kind.clone();
        entry.tcp_port = tcp_port;
        found_existing = true;
        entry
    });

    if found_existing {
        if let Some(entry) = state.peers.get(&peer_id) {
            let _ = app_handle.emit("peer-updated", entry.value());
        }
        return false; // peer existente, no nuevo
    }

    // Entrada nueva
    let peer_entry = PeerEntry {
        peer_id: peer_id.clone(),
        hostname: format!(
            "Device-{}",
            mac.replace([':', '-'], "")
                .get(0..6)
                .unwrap_or("unknown")
        ),
        ip: ip.to_string(),
        mac_address: Some(mac.to_string()),
        tcp_port,
        kind,
        last_seen: current_epoch(),
        online: true,
        app_version: None,
    };
    info!(
        "[scanner] Nuevo peer: {} ({}) — App: {}",
        peer_entry.hostname, ip, has_app
    );
    state.peers.insert(peer_id, peer_entry.clone());
    let _ = app_handle.emit("peer-updated", &peer_entry);
    true
}
