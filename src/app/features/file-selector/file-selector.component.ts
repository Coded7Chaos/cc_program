import { Component, inject, output, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { TauriBridgeService } from '../../core/services/tauri-bridge.service';
import { TransferService } from '../../core/services/transfer.service';
import { BytesFormatPipe } from '../../shared/pipes/bytes-format.pipe';
import { FileInfo } from '../../core/models/transfer.model';

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

  readonly fileInfo = signal<FileInfo | null>(null);
  readonly destinationPath = signal('C:\\Descargas');
  readonly loading = signal(false);
  readonly sending = signal(false);
  readonly error = signal<string | null>(null);

  targetPeerIds: string[] = [];

  transferStarted = output<string>();

  onPeersSelected(peerIds: string[]): void {
    this.targetPeerIds = peerIds;
  }

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
    if (!file || this.targetPeerIds.length === 0) return;

    this.sending.set(true);
    this.error.set(null);
    try {
      const transferId = await this.bridge.sendFile(
        file.path,
        this.destinationPath(),
        this.targetPeerIds
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
    return !!this.fileInfo() && this.targetPeerIds.length > 0 && !this.sending();
  }
}
