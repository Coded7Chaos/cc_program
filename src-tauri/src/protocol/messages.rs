use serde::{Deserialize, Serialize};

/// Tipos de mensajes UDP (broadcast discovery)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UdpMsgType {
    PeerAnnounce,
    PeerBye,
    ChunkAvailable,
}

/// Tipos de mensajes TCP (transferencia y coordinación P2P)
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
    /// BitTorrent-style "HAVE" message
    HaveChunk,
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

/// Notificación de chunks disponibles (receptor re-broadcast via UDP)
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

/// Estructura para describir a un vecino en el enjambre
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmPeer {
    pub peer_id: String,
    pub ip: String,
    pub tcp_port: u16,
}

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
    /// Lista de todas las IPs/Peers involucrados en esta transferencia (Enjambre)
    pub swarm: Vec<SwarmPeer>,
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

/// Mensaje "HAVE": indica que un peer ha completado un chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HaveChunk {
    pub msg_type: TcpMsgType,
    pub transfer_id: String,
    pub peer_id: String,
    /// IP y puerto del peer que anuncia. Necesarios porque el announce del sender
    /// lista a los receptores con el peer_id del scanner ("ip:puerto"), mientras que
    /// cada peer se identifica con su UUID propio: sin estos campos, el receptor de
    /// un HAVE no puede ubicar al emisor en el enjambre y nunca le pediría chunks.
    /// `default` para tolerar mensajes de versiones viejas de la app.
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub tcp_port: u16,
    pub chunk_index: u32,
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

// NOTA: NO usar un enum con #[serde(tag = "msg_type")] para despachar mensajes
// entrantes. Serde consume el campo tag al elegir la variante, y como los structs
// concretos también declaran msg_type como campo obligatorio, la deserialización
// interna falla con "missing field msg_type" para TODOS los mensajes. El despacho
// correcto es: deserializar TcpMsgEnvelope primero y re-parsear el struct completo
// desde los mismos bytes (ver receiver::handle_incoming_connection).

#[cfg(test)]
mod tests {
    use super::*;

    /// Protege el contrato de wire: el sender serializa los structs concretos
    /// (con su campo msg_type) y el receptor despacha leyendo primero el envelope
    /// y luego re-deserializando el struct completo desde los mismos bytes.
    #[test]
    fn envelope_dispatch_desde_struct_concreto() {
        let announce = TransferAnnounce {
            msg_type: TcpMsgType::TransferAnnounce,
            transfer_id: "t-1".into(),
            sender_peer_id: "uuid-sender".into(),
            sender_ip: "192.168.1.10".into(),
            file_name: "instalador.exe".into(),
            file_size: 2_097_152,
            total_chunks: 2,
            chunk_size: 1_048_576,
            chunk_hashes: vec!["aaa".into(), "bbb".into()],
            destination_path: "C:\\Descargas".into(),
            swarm: vec![],
        };
        let bytes = serde_json::to_vec(&announce).unwrap();

        let env: TcpMsgEnvelope = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(env.msg_type, TcpMsgType::TransferAnnounce);

        let parsed: TransferAnnounce = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.transfer_id, announce.transfer_id);
        assert_eq!(parsed.chunk_hashes, announce.chunk_hashes);
    }

    /// Igual que el anterior pero para HaveChunk, incluyendo compatibilidad con
    /// mensajes de versiones viejas que no traían ip/tcp_port.
    #[test]
    fn envelope_dispatch_have_chunk_y_compat() {
        let have = HaveChunk {
            msg_type: TcpMsgType::HaveChunk,
            transfer_id: "t-1".into(),
            peer_id: "uuid-r1".into(),
            ip: "192.168.1.20".into(),
            tcp_port: 47833,
            chunk_index: 3,
        };
        let bytes = serde_json::to_vec(&have).unwrap();
        let env: TcpMsgEnvelope = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(env.msg_type, TcpMsgType::HaveChunk);
        let parsed: HaveChunk = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.tcp_port, 47833);

        // Mensaje viejo sin ip/tcp_port: debe deserializar con defaults
        let viejo = r#"{"msg_type":"have_chunk","transfer_id":"t-1","peer_id":"p","chunk_index":0}"#;
        let parsed_viejo: HaveChunk = serde_json::from_str(viejo).unwrap();
        assert_eq!(parsed_viejo.ip, "");
        assert_eq!(parsed_viejo.tcp_port, 0);
    }
}
