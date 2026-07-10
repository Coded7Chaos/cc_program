import { Component, ElementRef, OnDestroy, OnInit, ViewChild, inject, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { TauriBridgeService } from '../../core/services/tauri-bridge.service';

/**
 * Visor de logs en vivo: muestra la cola del archivo app.log que escribe el
 * backend (tracing). Se refresca solo cada 3 segundos mientras está visible,
 * para poder diagnosticar transferencias sin abrir el Bloc de Notas.
 */
@Component({
  selector: 'app-log-viewer',
  standalone: true,
  imports: [CommonModule],
  templateUrl: './log-viewer.component.html',
  styleUrl: './log-viewer.component.css',
})
export class LogViewerComponent implements OnInit, OnDestroy {
  private bridge = inject(TauriBridgeService);

  readonly logs = signal<string>('');
  readonly logPath = signal<string>('');
  readonly autoRefresh = signal<boolean>(true);
  readonly loading = signal<boolean>(false);
  readonly lastUpdated = signal<Date | null>(null);

  @ViewChild('logContainer') logContainer?: ElementRef<HTMLPreElement>;

  private refreshTimer: ReturnType<typeof setInterval> | null = null;
  private readonly REFRESH_MS = 3000;
  private readonly MAX_LINES = 800;

  async ngOnInit(): Promise<void> {
    this.logPath.set(await this.bridge.getLogFilePath().catch(() => ''));
    await this.refresh();
    this.startTimer();
  }

  ngOnDestroy(): void {
    this.stopTimer();
  }

  async refresh(): Promise<void> {
    this.loading.set(true);
    try {
      const content = await this.bridge.getAppLogs(this.MAX_LINES);
      const wasAtBottom = this.isScrolledToBottom();
      this.logs.set(content);
      this.lastUpdated.set(new Date());
      // Mantener el scroll pegado al final solo si el usuario ya estaba ahí,
      // para no interrumpirlo si está leyendo líneas viejas.
      if (wasAtBottom) {
        setTimeout(() => this.scrollToBottom(), 0);
      }
    } catch (e) {
      this.logs.set(`No se pudieron leer los logs: ${e}`);
    } finally {
      this.loading.set(false);
    }
  }

  toggleAutoRefresh(): void {
    this.autoRefresh.update(v => !v);
    if (this.autoRefresh()) {
      this.startTimer();
    } else {
      this.stopTimer();
    }
  }

  private startTimer(): void {
    this.stopTimer();
    this.refreshTimer = setInterval(() => this.refresh(), this.REFRESH_MS);
  }

  private stopTimer(): void {
    if (this.refreshTimer !== null) {
      clearInterval(this.refreshTimer);
      this.refreshTimer = null;
    }
  }

  private isScrolledToBottom(): boolean {
    const el = this.logContainer?.nativeElement;
    if (!el) return true;
    return el.scrollHeight - el.scrollTop - el.clientHeight < 40;
  }

  private scrollToBottom(): void {
    const el = this.logContainer?.nativeElement;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
  }
}
