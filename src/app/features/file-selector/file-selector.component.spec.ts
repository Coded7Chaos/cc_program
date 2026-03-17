import { ComponentFixture, TestBed } from '@angular/core/testing';
import { FileSelectorComponent } from './file-selector.component';
import { TauriBridgeService } from '../../core/services/tauri-bridge.service';
import { TransferService } from '../../core/services/transfer.service';
import { signal } from '@angular/core';

describe('FileSelectorComponent - Botón de Envío', () => {
  let component: FileSelectorComponent;
  let fixture: ComponentFixture<FileSelectorComponent>;

  beforeEach(async () => {
    // Mock simple de los servicios
    const bridgeMock = {
      openFileDialog: jasmine.createSpy('openFileDialog'),
      getFileInfo: jasmine.createSpy('getFileInfo'),
      sendFile: jasmine.createSpy('sendFile').and.resolveTo('test-id')
    };
    const transferMock = {
      transfers: signal([])
    };

    await TestBed.configureTestingModule({
      imports: [FileSelectorComponent],
      providers: [
        { provide: TauriBridgeService, useValue: bridgeMock },
        { provide: TransferService, useValue: transferMock }
      ]
    }).compileComponents();

    fixture = TestBed.createComponent(FileSelectorComponent);
    component = fixture.componentInstance;
    fixture.detectChanges();
  });

  it('debe tener el botón deshabilitado inicialmente', () => {
    expect(component.canSend).toBeFalse();
  });

  it('debe tener el botón deshabilitado si solo hay archivo pero no peers', () => {
    component.fileInfo.set({ name: 'test.txt', path: '/path', size: 100, sha1: 'abc' });
    fixture.detectChanges();
    expect(component.canSend).toBeFalse();
  });

  it('debe activar el botón cuando hay archivo Y peers seleccionados', () => {
    // 1. Seleccionar archivo
    component.fileInfo.set({ name: 'test.txt', path: '/path', size: 100, sha1: 'abc' });
    
    // 2. Simular entrada de peers (usando set en el signal de entrada)
    fixture.componentRef.setInput('targetPeerIds', ['peer-1']);
    
    fixture.detectChanges();
    expect(component.canSend).toBeTrue();
  });

  it('debe deshabilitar el botón mientras se está realizando el envío', () => {
    component.fileInfo.set({ name: 'test.txt', path: '/path', size: 100, sha1: 'abc' });
    fixture.componentRef.setInput('targetPeerIds', ['peer-1']);
    
    component.sending.set(true);
    fixture.detectChanges();
    
    expect(component.canSend).toBeFalse();
  });
});
