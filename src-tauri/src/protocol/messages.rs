use serde::{Deserialize, Serialize};

/// Tipos de mensajes UDP (broadcast discovery)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UdpMsgType {
    PeerAnnounce,
    PeerBye,
    ChunkAvailable,
}

/// Tipos de mensajes TCP (transferencia)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TcpMsgType {
    TransferAnnounce,
    TransferAccepted,
    TransferRejected,
    ChunkRequest,
    ChunkResponse,
    TransferComplete,
    TransferError,
}

// ─── Mensajes UDP ─────────────────────────────────────────────────────────────

/// Anuncio de presencia de un peer con la app
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerAnnounce {
    pub msg_type: UdpMsgType,
    pub peer_id: String,
    pub hostname: String,
    pub ip: String,
    pub tcp_port: u16,
    pub app_version: String,
    pub timestamp: u64,
}

/// Notificación de salida de un peer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerBye {
    pub msg_type: UdpMsgType,
    pub peer_id: String,
    pub ip: String,
}

/// Notificación de chunks disponibles (receptor re-broadcast)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkAvailable {
    pub msg_type: UdpMsgType,
    pub transfer_id: String,
    pub peer_id: String,
    pub ip: String,
    pub tcp_port: u16,
    pub chunks: Vec<u32>,
}

// ─── Mensajes TCP ─────────────────────────────────────────────────────────────

/// Anuncio de transferencia del sender al receiver
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferAnnounce {
    pub msg_type: TcpMsgType,
    pub transfer_id: String,
    pub sender_peer_id: String,
    pub sender_ip: String,
    pub file_name: String,
    pub file_size: u64,
    pub total_chunks: u32,
    pub chunk_size: u32,
    pub chunk_hashes: Vec<String>,  // SHA1 hex por chunk
    pub destination_path: String,
}

/// Aceptación de transferencia
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferAccepted {
    pub msg_type: TcpMsgType,
    pub transfer_id: String,
    pub peer_id: String,
}

/// Rechazo de transferencia
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferRejected {
    pub msg_type: TcpMsgType,
    pub transfer_id: String,
    pub peer_id: String,
    pub reason: String,
}

/// Solicitud de un chunk específico
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRequest {
    pub msg_type: TcpMsgType,
    pub transfer_id: String,
    pub chunk_index: u32,
    pub requester_id: String,
}

/// Respuesta con datos del chunk (header; los bytes raw van en data_len del frame)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkResponse {
    pub msg_type: TcpMsgType,
    pub transfer_id: String,
    pub chunk_index: u32,
    pub sha1_hash: String,
    pub chunk_size: u32,
}

/// Transferencia completada exitosamente
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferComplete {
    pub msg_type: TcpMsgType,
    pub transfer_id: String,
    pub final_sha1: String,
}

/// Error durante la transferencia
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferError {
    pub msg_type: TcpMsgType,
    pub transfer_id: String,
    pub error: String,
}

// ─── Enum unificado para deserializar mensajes TCP ────────────────────────────

/// Envelope para deserializar cualquier mensaje TCP por su msg_type
#[derive(Debug, Deserialize)]
pub struct TcpMsgEnvelope {
    pub msg_type: TcpMsgType,
}

/// Enum que representa cualquier mensaje TCP deserializado
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "msg_type", rename_all = "snake_case")]
pub enum TcpMessage {
    TransferAnnounce(TransferAnnounce),
    TransferAccepted(TransferAccepted),
    TransferRejected(TransferRejected),
    ChunkRequest(ChunkRequest),
    ChunkResponse(ChunkResponse),
    TransferComplete(TransferComplete),
    TransferError(TransferError),
}
