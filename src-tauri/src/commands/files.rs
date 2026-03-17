use std::path::Path;
use tauri::{Emitter, AppHandle};
use sha1::{Digest, Sha1};
use std::fs::File;
use std::io::Read;
use tauri_plugin_dialog::DialogExt;

/// Información de un archivo seleccionado
#[derive(serde::Serialize)]
pub struct FileInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub sha1: String,
}

/// Abre el selector de archivos nativo y retorna la ruta seleccionada
#[tauri::command]
pub async fn open_file_dialog(app: AppHandle) -> Result<Option<String>, String> {
    let path = app
        .dialog()
        .file()
        .blocking_pick_file();

    Ok(path.map(|p| p.to_string()))
}

/// Retorna información de un archivo con progreso de hash
#[tauri::command]
pub async fn get_file_info(path: String, app: AppHandle) -> Result<FileInfo, String> {
    let file_path = Path::new(&path);
    if !file_path.exists() {
        return Err(format!("El archivo no existe: {}", path));
    }

    let name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("archivo").to_string();
    let size = std::fs::metadata(file_path).map_err(|e| format!("Error: {}", e))?.len();

    let path_buf = file_path.to_path_buf();
    let sha1 = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let mut file = File::open(&path_buf).map_err(|e| e.to_string())?;
        let mut hasher = Sha1::new();
        let mut buf = vec![0u8; 1024 * 1024]; // 1MB buffer
        let mut processed = 0u64;

        loop {
            let n = file.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 { break; }
            hasher.update(&buf[..n]);
            processed += n as u64;
            
            // Emitir progreso cada 10MB aproximadamente para no saturar el canal
            if processed % (10 * 1024 * 1024) == 0 || processed == size {
                let progress = (processed as f64 / size as f64) * 100.0;
                let _ = app.emit("hash-progress", progress);
            }
        }
        Ok(format!("{:x}", hasher.finalize()))
    }).await.map_err(|e| e.to_string())??;

    Ok(FileInfo { name, path, size, sha1 })
}
