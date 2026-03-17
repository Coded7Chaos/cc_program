import { Component, inject, input, output, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { TauriBridgeService } from '../../core/services/tauri-bridge.service';
import { TransferService } from '../../core/services/transfer.service';
import { PeerService } from '../../core/services/peer.service';
import { BytesFormatPipe } from '../../shared/pipes/bytes-format.pipe';
import { FileInfo } from '../../core/models/transfer.model';
import { ask } from '@tauri-apps/plugin-dialog';

@Component({
  selector: 'app-file-selector',
  standalone: true,
  imports: [CommonModule, FormsModule, BytesFormatPipe],
  templateUrl: './file-selector.component.html',
  styleUrl: './file-selector.component.css',
})
export class FileSelectorComponent {
  private bridge = inject(TauriBridgeService);
  private transferService = inject(TransferService);
  private peerService = inject(PeerService);

  readonly fileInfo = signal<FileInfo | null>(null);
  readonly destinationPath = signal('C:\\Descargas');
  readonly loading = signal(false);
  readonly sending = signal(false);
  readonly error = signal<string | null>(null);

  targetPeerIds = input<string[]>([]);

  transferStarted = output<string>();

  async selectFile(): Promise<void> {
    this.loading.set(true);
    this.error.set(null);
    try {
      const path = await this.bridge.openFileDialog();
      if (path) {
        const info = await this.bridge.getFileInfo(path);
        this.fileInfo.set(info);
      }
    } catch (e) {
      this.error.set(`Error al seleccionar archivo: ${e}`);
    } finally {
      this.loading.set(false);
    }
  }

  async send(): Promise<void> {
    const file = this.fileInfo();
    const selectedIds = this.targetPeerIds();
    if (!file || selectedIds.length === 0) return;

    // 1. Verificar cuáles de los seleccionados siguen disponibles y tienen la App
    const availablePeers = this.peerService.peers().filter(p => 
      selectedIds.includes(p.peer_id) && p.online && p.kind === 'App'
    );

    if (availablePeers.length === 0) {
      this.error.set('Ninguno de los peers seleccionados está disponible actualmente.');
      return;
    }

    // 2. Preparar mensaje de confirmación con la lista
    const peerListStr = availablePeers.map(p => `• ${p.hostname} (${p.ip})`).join('\n');
    const confirmed = await ask(
      `¿Está seguro de enviar el archivo "${file.name}" a estas computadoras?\n\n${peerListStr}`,
      { 
        title: 'Confirmar envío masivo',
        kind: 'info',
        okLabel: 'Sí, enviar',
        cancelLabel: 'Cancelar'
      }
    );

    if (!confirmed) return;

    this.sending.set(true);
    this.error.set(null);
    try {
      // Solo enviar a los que verificamos que están disponibles
      const finalPeerIds = availablePeers.map(p => p.peer_id);
      const transferId = await this.bridge.sendFile(
        file.path,
        this.destinationPath(),
        finalPeerIds
      );
      this.transferStarted.emit(transferId);
      this.fileInfo.set(null);
    } catch (e) {
      this.error.set(`Error al enviar: ${e}`);
    } finally {
      this.sending.set(false);
    }
  }

  get canSend(): boolean {
    return !!this.fileInfo() && this.targetPeerIds().length > 0 && !this.sending();
  }
}
