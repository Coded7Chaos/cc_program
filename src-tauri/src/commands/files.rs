use std::path::Path;
use std::sync::Arc;
use tauri::State;
use tauri_plugin_dialog::DialogExt;
use crate::state::AppState;
use crate::transfer::chunker::sha1_file;

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
pub async fn open_file_dialog(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let path = app
        .dialog()
        .file()
        .blocking_pick_file();

    Ok(path.map(|p| p.to_string()))
}

/// Retorna información de un archivo: nombre, tamaño y SHA1 completo
#[tauri::command]
pub async fn get_file_info(path: String) -> Result<FileInfo, String> {
    let file_path = Path::new(&path);

    let name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("archivo")
        .to_string();

    let size = std::fs::metadata(file_path)
        .map_err(|e| format!("Error al leer metadata: {}", e))?
        .len();

    // Calcular SHA1 en thread bloqueante para no bloquear el runtime
    let path_clone = file_path.to_path_buf();
    let sha1 = tokio::task::spawn_blocking(move || sha1_file(&path_clone))
        .await
        .map_err(|e| format!("Error en task: {}", e))?
        .map_err(|e| format!("Error calculando SHA1: {}", e))?;

    Ok(FileInfo { name, path, size, sha1 })
}
