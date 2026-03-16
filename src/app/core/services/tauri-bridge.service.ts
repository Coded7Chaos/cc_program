import { Injectable } from '@angular/core';
import { invoke } from '@tauri-apps/api/core';
import { listen, Event, UnlistenFn } from '@tauri-apps/api/event';
import { PeerEntry } from '../models/peer.model';
import { ActiveTransfer, FileInfo } from '../models/transfer.model';

@Injectable({ providedIn: 'root' })
export class TauriBridgeService {

  // ─── Peer Commands ──────────────────────────────────────────────────────────

  getPeers(): Promise<PeerEntry[]> {
    return invoke<PeerEntry[]>('get_peers');
  }

  refreshPeers(): Promise<void> {
    return invoke<void>('refresh_peers');
  }

  // ─── File Commands ──────────────────────────────────────────────────────────

  openFileDialog(): Promise<string | null> {
    return invoke<string | null>('open_file_dialog');
  }

  getFileInfo(path: string): Promise<FileInfo> {
    return invoke<FileInfo>('get_file_info', { path });
  }

  // ─── Transfer Commands ──────────────────────────────────────────────────────

  sendFile(path: string, destPath: string, targetPeerIds: string[]): Promise<string> {
    return invoke<string>('send_file', {
      path,
      destPath,
      targetPeerIds,
    });
  }

  getActiveTransfers(): Promise<ActiveTransfer[]> {
    return invoke<ActiveTransfer[]>('get_active_transfers');
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
}
