use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::protocol::messages::SwarmPeer;
use crate::transfer::tracker::TransferTracker;

/// Tipo de peer en la red
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PeerKind {
    /// Tiene la app instalada, descubierto via UDP
    App,
    /// PC en la red sin la app, descubierto via escaneo TCP:445
    NonApp,
}

/// Entrada de un peer en el registro
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerEntry {
    pub peer_id: String,
    pub hostname: String,
    pub ip: String,
    pub mac_address: Option<String>,
    pub tcp_port: u16,
    pub kind: PeerKind,
    pub last_seen: u64,  // epoch seconds
    pub online: bool,
    pub app_version: Option<String>,
}

/// Rol en una transferencia
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TransferRole {
    Sender,
    Receiver,
}

/// Estado de una transferencia
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TransferStatus {
    Pending,
    InProgress,
    Completed,
    Failed(String),
    Cancelled,
}

/// Transferencia activa (sender o receiver)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTransfer {
    pub transfer_id: String,
    pub file_name: String,
    pub file_path: String,
    pub file_size: u64,
    pub total_chunks: u32,
    pub chunk_size: u32,
    pub chunk_hashes: Vec<String>,  // SHA1 por chunk en hex
    pub destination_path: String,
    pub role: TransferRole,
    pub chunks_done: Vec<bool>,
    pub status: TransferStatus,
    pub target_peers: Vec<String>,  // peer_ids destino (para sender)
    pub swarm: Vec<SwarmPeer>,      // Peers en el enjambre
    pub sender_ip: String,
    pub sender_peer_id: String,
    pub bytes_transferred: u64,
    pub started_at: u64,  // epoch seconds
}

impl ActiveTransfer {
    pub fn chunks_completed(&self) -> u32 {
        self.chunks_done.iter().filter(|&&done| done).count() as u32
    }

    pub fn progress_percent(&self) -> f64 {
        if self.total_chunks == 0 {
            return 100.0;
        }
        (self.chunks_completed() as f64 / self.total_chunks as f64) * 100.0
    }
}

/// Configuración persistida de la aplicación
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub default_destination: String,
    pub max_concurrent_chunks: usize,
    pub auto_accept_transfers: bool,
    pub minimize_to_tray: bool,
    pub autostart: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_destination: "C:\\Descargas".to_string(),
            max_concurrent_chunks: 4,
            auto_accept_transfers: true,
            minimize_to_tray: true,
            autostart: false,
        }
    }
}

/// Estado compartido de la aplicación
pub struct AppState {
    pub peer_id: String,
    pub hostname: String,
    pub local_ip: String,
    pub peers: Arc<DashMap<String, PeerEntry>>,
    pub active_transfers: Arc<DashMap<String, ActiveTransfer>>,
    pub tracker: Arc<Mutex<TransferTracker>>,
    pub config: Arc<RwLock<AppConfig>>,
    pub shutdown_tx: broadcast::Sender<()>,
    /// Mapa transfer_id -> cancel_sender para cancelar transfers individuales
    pub cancel_senders: Arc<DashMap<String, tokio::sync::watch::Sender<bool>>>,
}

impl AppState {
    pub fn new(
        peer_id: String,
        hostname: String,
        local_ip: String,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Self {
        Self {
            peer_id,
            hostname,
            local_ip,
            peers: Arc::new(DashMap::new()),
            active_transfers: Arc::new(DashMap::new()),
            tracker: Arc::new(Mutex::new(TransferTracker::default())),
            config: Arc::new(RwLock::new(AppConfig::default())),
            shutdown_tx,
            cancel_senders: Arc::new(DashMap::new()),
        }
    }
}
