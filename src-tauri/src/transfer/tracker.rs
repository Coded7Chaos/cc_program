//! Tracker de chunks: registra qué peers tienen qué chunks disponibles en el enjambre.

use std::collections::HashMap;
use crate::protocol::messages::SwarmPeer;
use rand::seq::SliceRandom;

/// Entrada en el tracker para un transfer específico
#[derive(Debug, Default)]
pub struct TransferTrackerEntry {
    /// Lista completa de peers conocidos en el enjambre para esta transferencia
    pub swarm: Vec<SwarmPeer>,
    /// chunk_index -> Vec<peer_id>
    pub chunk_peers: HashMap<u32, Vec<String>>,
}

impl TransferTrackerEntry {
    /// Inicializa o actualiza la lista de peers del enjambre
    pub fn set_swarm(&mut self, swarm: Vec<SwarmPeer>) {
        self.swarm = swarm;
    }

    /// Registra que un peer tiene un chunk específico
    pub fn add_peer_chunk(&mut self, peer_id: String, chunk_idx: u32) {
        let peers = self.chunk_peers.entry(chunk_idx).or_default();
        if !peers.contains(&peer_id) {
            peers.push(peer_id);
        }
    }

    /// Retorna los peers (SwarmPeer completo) que tienen un chunk específico
    pub fn get_peers_for_chunk(&self, chunk_index: u32) -> Vec<SwarmPeer> {
        let peer_ids = match self.chunk_peers.get(&chunk_index) {
            Some(ids) => ids,
            None => return Vec::new(),
        };

        self.swarm
            .iter()
            .filter(|p| peer_ids.contains(&p.peer_id))
            .cloned()
            .collect()
    }

    /// Retorna un peer aleatorio del enjambre que tenga el chunk para balancear carga
    pub fn best_peer_for_chunk(&self, chunk_index: u32) -> Option<SwarmPeer> {
        let available_peers = self.get_peers_for_chunk(chunk_index);
        let mut rng = rand::thread_rng();
        available_peers.choose(&mut rng).cloned()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracker_swarm_and_chunks() {
        let mut tracker = TransferTracker::default();
        let tid = "test-transfer";
        let entry = tracker.get_or_create(tid);

        let peers = vec![
            SwarmPeer { peer_id: "p1".into(), ip: "1.1.1.1".into(), tcp_port: 80 },
            SwarmPeer { peer_id: "p2".into(), ip: "2.2.2.2".into(), tcp_port: 80 },
        ];
        entry.set_swarm(peers);

        // p1 tiene chunk 0
        entry.add_peer_chunk("p1".to_string(), 0);
        // p2 tiene chunk 0 y 1
        entry.add_peer_chunk("p2".to_string(), 0);
        entry.add_peer_chunk("p2".to_string(), 1);

        let chunk0_peers = entry.get_peers_for_chunk(0);
        assert_eq!(chunk0_peers.len(), 2);

        let chunk1_peers = entry.get_peers_for_chunk(1);
        assert_eq!(chunk1_peers.len(), 1);
        assert_eq!(chunk1_peers[0].peer_id, "p2");

        // best_peer para chunk 1 debe ser p2
        let best = entry.best_peer_for_chunk(1);
        assert_eq!(best.unwrap().peer_id, "p2");
    }
}
