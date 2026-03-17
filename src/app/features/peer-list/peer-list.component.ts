import { Component, inject, output, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { PeerService } from '../../core/services/peer.service';
import { PeerEntry } from '../../core/models/peer.model';

@Component({
  selector: 'app-peer-list',
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: './peer-list.component.html',
  styleUrl: './peer-list.component.css',
})
export class PeerListComponent {
  readonly peerService = inject(PeerService);
  readonly selectedPeerIds = signal<Set<string>>(new Set());

  peersSelected = output<string[]>();

  togglePeer(peerId: string, event: Event): void {
    const checked = (event.target as HTMLInputElement).checked;
    this.selectedPeerIds.update(ids => {
      const newIds = new Set(ids);
      if (checked) {
        newIds.add(peerId);
      } else {
        newIds.delete(peerId);
      }
      return newIds;
    });
    this.peersSelected.emit(Array.from(this.selectedPeerIds()));
  }

  toggleAll(event: Event): void {
    const checked = (event.target as HTMLInputElement).checked;
    if (checked) {
      // Solo seleccionar aquellos que son tipo 'App'
      const appPeerIds = this.peerService.peers()
        .filter(p => p.kind === 'App')
        .map(p => p.peer_id);
      this.selectedPeerIds.set(new Set(appPeerIds));
    } else {
      this.selectedPeerIds.set(new Set());
    }
    this.peersSelected.emit(Array.from(this.selectedPeerIds()));
  }

  isSelected(peerId: string): boolean {
    return this.selectedPeerIds().has(peerId);
  }

  async refresh(): Promise<void> {
    await this.peerService.refresh();
  }

  trackByPeer(_: number, peer: PeerEntry): string {
    return peer.peer_id;
  }
}
