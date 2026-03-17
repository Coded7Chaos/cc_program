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
  readonly checking = signal(false);
  readonly sending = signal(false);
  readonly error = signal<string | null>(null);

  targetPeerIds = input<string[]>([]);

  transferStarted = output<string>();

  async selectFile(): Promise<void> {
    console.group('[P2P] Selección de archivo');
    this.loading.set(true);
    this.error.set(null);
    try {
      const path = await this.bridge.openFileDialog();
      if (path) {
        console.log('Ruta seleccionada:', path);
        console.log('Calculando hashes SHA1 de chunks...');
        const t0 = performance.now();
        const info = await this.bridge.getFileInfo(path);
        console.log(`Hash calculado en ${(performance.now() - t0).toFixed(0)}ms:`, info);
        this.fileInfo.set(info);
      } else {
        console.log('Selección cancelada por el usuario');
      }
    } catch (e) {
      console.error('Error al seleccionar archivo:', e);
      this.error.set(`Error al seleccionar archivo: ${e}`);
    } finally {
      this.loading.set(false);
      console.groupEnd();
    }
  }

  async send(): Promise<void> {
    const file = this.fileInfo();
    const selectedIds = this.targetPeerIds();

    console.group('[P2P] Flujo de envío iniciado');
    console.log('Archivo:', file?.name, `(${file?.size} bytes)`);
    console.log('Peers seleccionados:', selectedIds.length, selectedIds);

    if (!file || selectedIds.length === 0) {
      console.warn('Abortado: falta archivo o peers seleccionados');
      console.groupEnd();
      return;
    }

    // 1. Verificación TCP en vivo de cada peer seleccionado
    console.group('[P2P] Paso 1 — Verificación en vivo (TCP connect)');
    this.checking.set(true);
    this.error.set(null);
    let onlinePeerIds: string[] = [];
    try {
      const t0 = performance.now();
      onlinePeerIds = await this.bridge.checkPeersOnline(selectedIds);
      const elapsed = (performance.now() - t0).toFixed(0);
      console.log(`Verificación completada en ${elapsed}ms`);
      console.log('Respondieron TCP:', onlinePeerIds.length, onlinePeerIds);
      const offlineIds = selectedIds.filter(id => !onlinePeerIds.includes(id));
      if (offlineIds.length > 0) {
        console.warn('No respondieron (sin app o apagadas):', offlineIds);
      }
    } catch (e) {
      console.error('Error en verificación TCP, usando caché:', e);
      onlinePeerIds = this.peerService.peers()
        .filter(p => selectedIds.includes(p.peer_id) && p.online)
        .map(p => p.peer_id);
      console.log('Peers del caché como fallback:', onlinePeerIds);
    } finally {
      this.checking.set(false);
      console.groupEnd();
    }

    // 2. Filtrar a los que tienen la App y respondieron
    console.group('[P2P] Paso 2 — Filtrar peers con App instalada');
    const allPeers = this.peerService.peers();
    const availablePeers = allPeers.filter(p =>
      onlinePeerIds.includes(p.peer_id) && p.kind === 'App'
    );
    const nonAppOnline = allPeers.filter(p =>
      onlinePeerIds.includes(p.peer_id) && p.kind !== 'App'
    );
    if (nonAppOnline.length > 0) {
      console.warn('Respondieron pero sin App instalada (se ignoran):', nonAppOnline.map(p => p.ip));
    }
    console.log('Destinos válidos:', availablePeers.length);
    if (availablePeers.length > 0) {
      console.table(availablePeers.map(p => ({ hostname: p.hostname, ip: p.ip, port: p.tcp_port, peer_id: p.peer_id })));
    }
    console.groupEnd();

    if (availablePeers.length === 0) {
      console.error('Abortado: ninguna computadora disponible con la App');
      this.error.set('Ninguna de las computadoras seleccionadas está disponible en este momento.');
      console.groupEnd();
      return;
    }

    // 3. Diálogo de confirmación
    console.group('[P2P] Paso 3 — Confirmación del usuario');
    const peerListStr = availablePeers.map(p => `• ${p.hostname} (${p.ip})`).join('\n');
    console.log('Mostrando diálogo para:', availablePeers.map(p => p.hostname).join(', '));

    let confirmed = false;
    if ((window as any).__TAURI_INTERNALS__) {
      confirmed = await ask(
        `¿Está seguro de enviar el archivo "${file.name}" a estas computadoras?\n\n${peerListStr}`,
        {
          title: 'Confirmar envío masivo',
          kind: 'info',
          okLabel: 'Sí, enviar',
          cancelLabel: 'Cancelar'
        }
      );
    } else {
      confirmed = confirm(`¿Enviar "${file.name}" a:\n${peerListStr}?`);
    }
    console.log('Respuesta del usuario:', confirmed ? 'CONFIRMADO' : 'CANCELADO');
    console.groupEnd();

    if (!confirmed) {
      console.log('Envío cancelado por el usuario');
      console.groupEnd();
      return;
    }

    // 4. Invocar comando Rust para iniciar el enjambre P2P
    console.group('[P2P] Paso 4 — Iniciando enjambre BitTorrent');
    this.sending.set(true);
    this.error.set(null);
    try {
      const finalPeerIds = availablePeers.map(p => p.peer_id);
      console.log('Invocando send_file en backend Rust...');
      console.log('  archivo:', file.path);
      console.log('  destino:', this.destinationPath());
      console.log('  peer_ids:', finalPeerIds);
      const t0 = performance.now();
      const transferId = await this.bridge.sendFile(
        file.path,
        this.destinationPath(),
        finalPeerIds
      );
      console.log(`send_file completado en ${(performance.now() - t0).toFixed(0)}ms`);
      console.log('Transfer ID asignado:', transferId);
      this.transferStarted.emit(transferId);
      this.fileInfo.set(null);
    } catch (e) {
      console.error('Error al iniciar el enjambre P2P:', e);
      this.error.set(`Error al enviar: ${e}`);
    } finally {
      this.sending.set(false);
      console.groupEnd();
      console.groupEnd(); // cierra el grupo principal
    }
  }

  get canSend(): boolean {
    return !!this.fileInfo() && this.targetPeerIds().length > 0 && !this.sending() && !this.checking();
  }
}
