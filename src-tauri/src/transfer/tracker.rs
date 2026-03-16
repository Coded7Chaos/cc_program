//! Tracker de chunks: registra qué peers tienen qué chunks disponibles.

use std::collections::HashMap;

/// Entrada en el tracker para un transfer específico
#[derive(Debug, Default)]
pub struct TransferTrackerEntry {
    /// chunk_index -> Vec<peer_ip>
    pub chunk_peers: HashMap<u32, Vec<String>>,
}

impl TransferTrackerEntry {
    /// Registra que un peer tiene los chunks dados
    pub fn add_peer_chunks(&mut self, peer_ip: String, chunks: Vec<u32>) {
        for chunk_idx in chunks {
            self.chunk_peers
                .entry(chunk_idx)
                .or_default()
                .push(peer_ip.clone());
        }
    }

    /// Retorna los peers que tienen un chunk específico
    pub fn get_peers_for_chunk(&self, chunk_index: u32) -> Vec<String> {
        self.chunk_peers
            .get(&chunk_index)
            .cloned()
            .unwrap_or_default()
    }

    /// Retorna el peer menos cargado que tiene el chunk (simple round-robin)
    pub fn best_peer_for_chunk(&self, chunk_index: u32) -> Option<String> {
        self.chunk_peers
            .get(&chunk_index)
            .and_then(|peers| peers.first().cloned())
    }
}

/// Tracker global: transfer_id -> TrackerEntry
#[derive(Debug, Default)]
pub struct TransferTracker {
    pub entries: HashMap<String, TransferTrackerEntry>,
}

impl TransferTracker {
    pub fn get_or_create(&mut self, transfer_id: &str) -> &mut TransferTrackerEntry {
        self.entries
            .entry(transfer_id.to_string())
            .or_default()
    }

    pub fn remove(&mut self, transfer_id: &str) {
        self.entries.remove(transfer_id);
    }
}
