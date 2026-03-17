import { Injectable, signal, computed, OnDestroy } from '@angular/core';
import { PeerEntry } from '../models/peer.model';
import { TauriBridgeService } from './tauri-bridge.service';
import { UnlistenFn } from '@tauri-apps/api/event';

@Injectable({ providedIn: 'root' })
export class PeerService implements OnDestroy {
  private readonly _peers = signal<PeerEntry[]>([]);
  private readonly _scanning = signal(false);
  private readonly _scanProgress = signal({ scanned: 0, total: 0, found: 0 });

  readonly peers = this._peers.asReadonly();
  readonly scanning = this._scanning.asReadonly();
  readonly scanProgress = this._scanProgress.asReadonly();

  readonly appPeers = computed(() => this._peers().filter(p => p.kind === 'App'));
  readonly nonAppPeers = computed(() => this._peers().filter(p => p.kind === 'NonApp'));
  readonly onlinePeers = computed(() => this._peers().filter(p => p.online));

  private unlistenFns: UnlistenFn[] = [];

  constructor(private bridge: TauriBridgeService) {
    this.init();
  }

  private async init(): Promise<void> {
    // Cargar peers existentes
    const existing = await this.bridge.getPeers().catch(() => []);
    this._peers.set(existing);

    // Escuchar actualizaciones
    const unlistenUpdated = await this.bridge.onPeerUpdated((peer) => {
      this._peers.update(peers => {
        const idx = peers.findIndex(p => p.peer_id === peer.peer_id);
        if (idx >= 0) {
          const updated = [...peers];
          updated[idx] = peer;
          return updated;
        }
        return [...peers, peer];
      });
    });

    const unlistenRemoved = await this.bridge.onPeerRemoved((peerId) => {
      this._peers.update(peers => peers.filter(p => p.peer_id !== peerId));
    });

    const unlistenScan = await this.bridge.onScanProgress((progress) => {
      this._scanProgress.set(progress);
      if (progress.scanned >= progress.total) {
        this._scanning.set(false);
      }
    });

    this.unlistenFns.push(unlistenUpdated, unlistenRemoved, unlistenScan);
  }

  async refresh(): Promise<void> {
    this._scanning.set(true);
    await this.bridge.refreshPeers().catch(console.error);
    // El scan-progress event apagará el spinner
    setTimeout(() => this._scanning.set(false), 10000);
  }

  async clearAll(): Promise<void> {
    await this.bridge.clearPeers().catch(console.error);
    // Los eventos peer-removed del backend limpiarán la lista,
    // pero también limpiamos localmente por si acaso
    this._peers.set([]);
  }

  ngOnDestroy(): void {
    this.unlistenFns.forEach(fn => fn());
  }
}
