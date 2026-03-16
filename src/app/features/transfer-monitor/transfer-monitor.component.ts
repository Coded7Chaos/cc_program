import { Component, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { TransferService } from '../../core/services/transfer.service';
import { ActiveTransfer, TransferStatus } from '../../core/models/transfer.model';
import { BytesFormatPipe } from '../../shared/pipes/bytes-format.pipe';

@Component({
  selector: 'app-transfer-monitor',
  standalone: true,
  imports: [CommonModule, BytesFormatPipe],
  templateUrl: './transfer-monitor.component.html',
  styleUrl: './transfer-monitor.component.css',
})
export class TransferMonitorComponent {
  readonly transferService = inject(TransferService);

  getProgress(transfer: ActiveTransfer): number {
    if (transfer.total_chunks === 0) return 100;
    const done = transfer.chunks_done.filter(Boolean).length;
    return Math.round((done / transfer.total_chunks) * 100);
  }

  getStatusLabel(status: TransferStatus): string {
    if (status === 'Pending') return 'Pendiente';
    if (status === 'InProgress') return 'En progreso';
    if (status === 'Completed') return 'Completado';
    if (status === 'Cancelled') return 'Cancelado';
    if (typeof status === 'object' && 'Failed' in status) return `Error: ${status.Failed}`;
    return String(status);
  }

  getStatusClass(status: TransferStatus): string {
    if (status === 'Completed') return 'status--completed';
    if (status === 'Cancelled') return 'status--cancelled';
    if (typeof status === 'object' && 'Failed' in status) return 'status--error';
    if (status === 'InProgress') return 'status--active';
    return 'status--pending';
  }

  isActive(status: TransferStatus): boolean {
    return status === 'InProgress' || status === 'Pending';
  }

  async cancel(transferId: string): Promise<void> {
    await this.transferService.cancel(transferId);
  }

  getChunksDone(transfer: ActiveTransfer): number {
    return transfer.chunks_done.filter(Boolean).length;
  }

  trackByTransfer(_: number, transfer: ActiveTransfer): string {
    return transfer.transfer_id;
  }
}
