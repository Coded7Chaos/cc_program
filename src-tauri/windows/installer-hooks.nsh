; Hooks de instalación NSIS (Tauri 2) — referenciado desde tauri.conf.json
; en bundle > windows > nsis > installerHooks.
;
; ¿Por qué existe este archivo?
; Windows Defender Firewall bloquea TODAS las conexiones TCP entrantes por defecto.
; Esta app necesita recibir conexiones entrantes en CADA máquina (el descubrimiento
; sondea el puerto 47833+, el sender recibe pedidos de chunks, los receptores reciben
; announces y HAVEs). Una app de escritorio NO puede autoconcederse la excepción de
; firewall en runtime sin elevación; el lugar correcto es el instalador, que con
; installMode "perMachine" corre elevado (UAC) y puede ejecutar netsh.

!macro NSIS_HOOK_POSTINSTALL
  ; Borrar regla previa para que reinstalaciones/updates no dupliquen
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="P2P File Deployer"'
  ; Permitir conexiones entrantes al ejecutable en TODOS los perfiles de red
  ; (Domain/Private/Public). profile=any es imprescindible: las redes de laboratorio
  ; suelen quedar clasificadas como "Public" y una regla solo-Private no aplicaría.
  ; Se permite el programa (no un puerto fijo) porque el listener se desplaza de
  ; puerto si 47833 está ocupado.
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="P2P File Deployer" dir=in action=allow program="$INSTDIR\${MAINBINARYNAME}.exe" enable=yes profile=any'

  ; Carpeta de destino por defecto de la app (AppConfig.default_destination),
  ; escribible por cualquier usuario aunque la app corra sin privilegios.
  ; *S-1-5-32-545 = grupo "Users" (independiente del idioma de Windows);
  ; (OI)(CI)M = permiso Modificar heredado por archivos y subcarpetas.
  nsExec::ExecToLog 'cmd /C if not exist "C:\Descargas" mkdir "C:\Descargas"'
  nsExec::ExecToLog 'icacls "C:\Descargas" /grant *S-1-5-32-545:(OI)(CI)M'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="P2P File Deployer"'
!macroend
