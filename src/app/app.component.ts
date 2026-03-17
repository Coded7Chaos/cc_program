import { Component, inject, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { RouterOutlet } from '@angular/router';
import { PeerListComponent } from './features/peer-list/peer-list.component';
import { FileSelectorComponent } from './features/file-selector/file-selector.component';
import { TransferMonitorComponent } from './features/transfer-monitor/transfer-monitor.component';
import { PeerService } from './core/services/peer.service';
import { TransferService } from './core/services/transfer.service';
import { NotificationService } from './core/services/notification.service';
import { TauriBridgeService } from './core/services/tauri-bridge.service';

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [
    CommonModule,
    RouterOutlet,
    PeerListComponent,
    FileSelectorComponent,
    TransferMonitorComponent,
  ],
  templateUrl: './app.component.html',
  styleUrl: './app.component.css',
})
export class AppComponent implements OnInit {
  private bridge = inject(TauriBridgeService);
  private notificationService = inject(NotificationService);
  readonly peerService = inject(PeerService);
  readonly transferService = inject(TransferService);

  readonly activeTab = signal<'peers' | 'transfers' | 'settings'>('peers');
  readonly selectedPeerIds = signal<string[]>([]);
  readonly tcpPort = signal<number | null>(null);
  readonly localPeerId = signal<string | null>(null);

  async ngOnInit(): Promise<void> {
    await this.notificationService.init();
    await this.fetchTcpPort();
    await this.fetchLocalPeerId();

    // Escuchar transferencias completadas para notificar
    await this.bridge.onTransferComplete(async (transferId) => {
      const transfers = await this.bridge.getActiveTransfers().catch(() => []);
      const transfer = transfers.find(t => t.transfer_id === transferId);
      if (transfer?.role === 'Receiver') {
        await this.notificationService.notifyTransferComplete(transfer.file_name);
      }
    });

    // Escuchar transferencias entrantes para notificar
    await this.bridge.onTransferIncoming(async (transfer) => {
      await this.notificationService.notifyTransferIncoming(transfer.file_name, transfer.sender_ip);
    });
  }

  async fetchTcpPort(): Promise<void> {
    const port = await this.bridge.getTcpPort();
    this.tcpPort.set(port);
  }

  async fetchLocalPeerId(): Promise<void> {
    const id = await this.bridge.getLocalPeerId();
    this.localPeerId.set(id);
  }

  onPeersSelected(peerIds: string[]): void {
    this.selectedPeerIds.set(peerIds);
  }

  onTransferStarted(transferId: string): void {
    this.activeTab.set('transfers');
  }

  setTab(tab: 'peers' | 'transfers' | 'settings'): void {
    this.activeTab.set(tab);
  }

  get transferCount(): number {
    return this.transferService.transfers().filter(
      t => t.status === 'InProgress' || t.status === 'Pending'
    ).length;
  }
}
