import { Injectable } from '@angular/core';
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from '@tauri-apps/plugin-notification';

@Injectable({ providedIn: 'root' })
export class NotificationService {
  private permissionGranted = false;

  async init(): Promise<void> {
    this.permissionGranted = await isPermissionGranted();
    if (!this.permissionGranted) {
      const permission = await requestPermission();
      this.permissionGranted = permission === 'granted';
    }
  }

  async send(title: string, body: string): Promise<void> {
    if (!this.permissionGranted) {
      await this.init();
    }
    if (this.permissionGranted) {
      sendNotification({ title, body });
    }
  }

  async notifyTransferComplete(fileName: string): Promise<void> {
    await this.send('Transferencia completada', `"${fileName}" ha sido recibido correctamente.`);
  }

  async notifyTransferIncoming(fileName: string, senderIp: string): Promise<void> {
    await this.send('Nueva transferencia', `Recibiendo "${fileName}" de ${senderIp}`);
  }
}
