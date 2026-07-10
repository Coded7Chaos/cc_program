//! Comandos para la pantalla de logs de la UI: leen el mismo archivo
//! que escribe el subscriber de tracing (ver lib.rs::init_logging).

/// Retorna las últimas `max_lines` líneas del archivo de log (default 500).
/// Se usa from_utf8_lossy porque el archivo podría contener bytes inválidos
/// si el proceso murió a mitad de una escritura.
#[tauri::command]
pub async fn get_app_logs(max_lines: Option<usize>) -> Result<String, String> {
    let Some(path) = crate::get_log_path() else {
        return Err("No se pudo determinar la ruta del archivo de log".to_string());
    };

    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| format!("No se pudo leer {}: {}", path, e))?;

    let content = String::from_utf8_lossy(&bytes);
    let max = max_lines.unwrap_or(500);
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(max);
    Ok(lines[start..].join("\n"))
}

/// Retorna la ruta absoluta del archivo de log, para mostrarla en la UI.
#[tauri::command]
pub fn get_log_file_path() -> Result<String, String> {
    crate::get_log_path()
        .ok_or_else(|| "No se pudo determinar la ruta del archivo de log".to_string())
}
