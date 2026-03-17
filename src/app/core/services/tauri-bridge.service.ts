import { Injectable } from '@angular/core';
import { invoke } from '@tauri-apps/api/core';
import { listen, Event, UnlistenFn } from '@tauri-apps/api/event';
import { PeerEntry } from '../models/peer.model';
import { ActiveTransfer, FileInfo } from '../models/transfer.model';

@Injectable({ providedIn: 'root' })
export class TauriBridgeService {

  private isTauri(): boolean {
    return !!(window as any).__TAURI_INTERNALS__;
  }

  // ─── Peer Commands ──────────────────────────────────────────────────────────

  async getPeers(): Promise<PeerEntry[]> {
    if (!this.isTauri()) return [];
    return invoke<PeerEntry[]>('get_peers');
  }

  async refreshPeers(): Promise<void> {
    if (!this.isTauri()) return;
    return invoke<void>('refresh_peers');
  }

  async checkPeersOnline(peerIds: string[]): Promise<string[]> {
    if (!this.isTauri()) return peerIds; // En modo browser asumir todos online
    return invoke<string[]>('check_peers_online', { peerIds });
  }

  async clearPeers(): Promise<void> {
    if (!this.isTauri()) return;
    return invoke<void>('clear_peers');
  }

  async getLocalPeerId(): Promise<string> {
    if (!this.isTauri()) return 'browser-mode';
    return invoke<string>('get_local_peer_id');
  }

  // ─── File Commands ──────────────────────────────────────────────────────────

  async openFileDialog(): Promise<string | null> {
    if (!this.isTauri()) {
      alert('Esta función solo está disponible en la versión de escritorio.');
      return null;
    }
    return invoke<string | null>('open_file_dialog');
  }

  async getFileInfo(path: string): Promise<FileInfo> {
    if (!this.isTauri()) throw new Error('Not in Tauri');
    return invoke<FileInfo>('get_file_info', { path });
  }

  // ─── Transfer Commands ──────────────────────────────────────────────────────

  async sendFile(path: string, destPath: string, targetPeerIds: string[]): Promise<string> {
    if (!this.isTauri()) throw new Error('Not in Tauri');
    return invoke<string>('send_file', {
      path,
      destPath,
      targetPeerIds,
    });
  }

  async getActiveTransfers(): Promise<ActiveTransfer[]> {
    if (!this.isTauri()) return [];
    return invoke<ActiveTransfer[]>('get_active_transfers');
  }

  async getTcpPort(): Promise<number> {
    if (!this.isTauri()) return 0;
    return invoke<number>('get_app_tcp_port');
  }

  cancelTransfer(transferId: string): Promise<void> {
    return invoke<void>('cancel_transfer', { transferId });
  }

  // ─── Event Listeners ────────────────────────────────────────────────────────

  onPeerUpdated(callback: (peer: PeerEntry) => void): Promise<UnlistenFn> {
    return listen<PeerEntry>('peer-updated', (event: Event<PeerEntry>) => {
      callback(event.payload);
    });
  }

  onPeerRemoved(callback: (peerId: string) => void): Promise<UnlistenFn> {
    return listen<string>('peer-removed', (event: Event<string>) => {
      callback(event.payload);
    });
  }

  onTransferIncoming(callback: (transfer: ActiveTransfer) => void): Promise<UnlistenFn> {
    return listen<ActiveTransfer>('transfer-incoming', (event: Event<ActiveTransfer>) => {
      callback(event.payload);
    });
  }

  onTransferProgress(callback: (payload: any) => void): Promise<UnlistenFn> {
    return listen<any>('transfer-progress', (event: Event<any>) => {
      callback(event.payload);
    });
  }

  onTransferComplete(callback: (transferId: string) => void): Promise<UnlistenFn> {
    return listen<string>('transfer-complete', (event: Event<string>) => {
      callback(event.payload);
    });
  }

  onTransferError(callback: (payload: { transfer_id: string; error: string }) => void): Promise<UnlistenFn> {
    return listen<{ transfer_id: string; error: string }>('transfer-error', (event) => {
      callback(event.payload);
    });
  }

  onScanProgress(callback: (payload: { scanned: number; total: number; found: number }) => void): Promise<UnlistenFn> {
    return listen<{ scanned: number; total: number; found: number }>('scan-progress', (event) => {
      callback(event.payload);
    });
  }

  onHashProgress(callback: (progress: number) => void): Promise<UnlistenFn> {
    return listen<number>('hash-progress', (event) => {
      callback(event.payload);
    });
  }
}
