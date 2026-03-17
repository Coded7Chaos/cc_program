import { Injectable, signal, OnDestroy } from '@angular/core';
import { ActiveTransfer, TransferProgressPayload } from '../models/transfer.model';
import { TauriBridgeService } from './tauri-bridge.service';
import { UnlistenFn } from '@tauri-apps/api/event';

@Injectable({ providedIn: 'root' })
export class TransferService implements OnDestroy {
  private readonly _transfers = signal<ActiveTransfer[]>([]);
  private readonly _speeds = signal<Map<string, number>>(new Map());
  private readonly _isPreparing = signal(false);
  private readonly _prepareProgress = signal(0);

  readonly transfers = this._transfers.asReadonly();
  readonly speeds = this._speeds.asReadonly();
  readonly isPreparing = this._isPreparing.asReadonly();
  readonly prepareProgress = this._prepareProgress.asReadonly();

  private unlistenFns: UnlistenFn[] = [];

  constructor(private bridge: TauriBridgeService) {
    this.init();
  }

  private async init(): Promise<void> {
    const existing = await this.bridge.getActiveTransfers().catch(() => []);
    this._transfers.set(existing);

    const unlistenHash = await this.bridge.onHashProgress((progress) => {
      this._isPreparing.set(progress < 100);
      this._prepareProgress.set(progress);
    });

    const unlistenIncoming = await this.bridge.onTransferIncoming((transfer) => {
      this._transfers.update(ts => [...ts, transfer]);
    });

    const unlistenProgress = await this.bridge.onTransferProgress((payload: TransferProgressPayload) => {
      this._transfers.update(ts =>
        ts.map(t =>
          t.transfer_id === payload.transfer_id
            ? {
                ...t,
                bytes_transferred: payload.bytes_transferred,
                status: payload.status,
                chunks_done: t.chunks_done.map((done, i) => done || i < payload.chunks_completed),
              }
            : t
        )
      );
      // Actualizar velocidad
      this._speeds.update(speeds => {
        const newSpeeds = new Map(speeds);
        newSpeeds.set(payload.transfer_id, payload.speed_bps);
        return newSpeeds;
      });
    });

    const unlistenComplete = await this.bridge.onTransferComplete((transferId) => {
      this._transfers.update(ts =>
        ts.map(t =>
          t.transfer_id === transferId ? { ...t, status: 'Completed' } : t
        )
      );
    });

    const unlistenError = await this.bridge.onTransferError((payload) => {
      this._transfers.update(ts =>
        ts.map(t =>
          t.transfer_id === payload.transfer_id
            ? { ...t, status: { Failed: payload.error } }
            : t
        )
      );
    });

    this.unlistenFns.push(unlistenHash, unlistenIncoming, unlistenProgress, unlistenComplete, unlistenError);
  }

  async cancel(transferId: string): Promise<void> {
    await this.bridge.cancelTransfer(transferId);
    this._transfers.update(ts =>
      ts.map(t =>
        t.transfer_id === transferId ? { ...t, status: 'Cancelled' } : t
      )
    );
  }

  getSpeed(transferId: string): number {
    return this._speeds().get(transferId) ?? 0;
  }

  ngOnDestroy(): void {
    this.unlistenFns.forEach(fn => fn());
  }
}
