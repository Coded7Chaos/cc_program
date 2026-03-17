//! Fragmentación de archivos en chunks de 512 KB con verificación SHA1.

use sha1::{Digest, Sha1};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use thiserror::Error;

pub const CHUNK_SIZE: u32 = 1024 * 1024; // 1 MB

#[derive(Debug, Error)]
pub enum ChunkerError {
    #[error("Error de IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("Índice de chunk inválido: {0}")]
    InvalidChunkIndex(u32),
    #[error("Hash mismatch: esperado {expected}, obtenido {actual}")]
    HashMismatch { expected: String, actual: String },
}

/// Calcula cuántos chunks necesita un archivo del tamaño dado
pub fn chunk_count(file_size: u64, chunk_size: u32) -> u32 {
    let chunk_size = chunk_size as u64;
    ((file_size + chunk_size - 1) / chunk_size) as u32
}

/// Calcula el SHA1 hex de un slice de bytes
pub fn sha1_hex(data: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Calcula el SHA1 de todo el archivo (para verificación final)
pub fn sha1_file(path: &Path) -> Result<String, ChunkerError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024]; // leer de a 64 KB
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Construye el mapa de hashes: Vec<sha1_hex> con un elemento por chunk
pub fn build_chunk_map(
    path: &std::path::Path, 
    chunk_size: u32,
    app: Option<tauri::AppHandle>
) -> Result<Vec<String>, ChunkerError> {
    let file_size = std::fs::metadata(path)?.len();
    let count = chunk_count(file_size, chunk_size);
    let mut file = File::open(path)?;
    let mut hashes = Vec::with_capacity(count as usize);

    for i in 0..count {
        let mut buf = vec![0u8; chunk_size as usize];
        let n = file.read(&mut buf)?;
        buf.truncate(n);
        hashes.push(sha1_hex(&buf));

        if let Some(ref h) = app {
            if i % 10 == 0 || i == count - 1 {
                let progress = ((i + 1) as f64 / count as f64) * 100.0;
                let _ = tauri::Emitter::emit(h, "hash-progress", progress);
            }
        }
    }

    Ok(hashes)
}

/// Lee un chunk específico del archivo por índice (offset calculado automáticamente)
pub fn read_chunk(
    path: &Path,
    chunk_index: u32,
    chunk_size: u32,
    expected_hash: &str,
) -> Result<Vec<u8>, ChunkerError> {
    let mut file = File::open(path)?;
    let offset = chunk_index as u64 * chunk_size as u64;
    file.seek(SeekFrom::Start(offset))?;

    let mut buf = vec![0u8; chunk_size as usize];
    let n = file.read(&mut buf)?;
    buf.truncate(n);

    // Verificar integridad
    let actual_hash = sha1_hex(&buf);
    if actual_hash != expected_hash {
        return Err(ChunkerError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    Ok(buf)
}

/// Verifica un chunk recibido contra su hash esperado
pub fn verify_chunk(data: &[u8], expected_hash: &str) -> Result<(), ChunkerError> {
    let actual = sha1_hex(data);
    if actual != expected_hash {
        return Err(ChunkerError::HashMismatch {
            expected: expected_hash.to_string(),
            actual,
        });
    }
    Ok(())
}

/// Escribe un chunk en la ruta destino al offset correspondiente
pub fn write_chunk_at_offset(
    dest_path: &Path,
    chunk_index: u32,
    chunk_size: u32,
    data: &[u8],
) -> Result<(), ChunkerError> {
    use std::io::Write;
    use std::fs::OpenOptions;

    let offset = chunk_index as u64 * chunk_size as u64;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .open(dest_path)?;

    file.seek(SeekFrom::Start(offset))?;
    file.write_all(data)?;
    Ok(())
}

/// Crea un archivo de destino con el tamaño correcto (pre-allocate)
pub fn preallocate_file(dest_path: &Path, file_size: u64) -> Result<(), ChunkerError> {
    use std::fs::OpenOptions;

    // Crear directorios padre si no existen
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(dest_path)?;

    file.set_len(file_size)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_chunk_count() {
        assert_eq!(chunk_count(0, 1024), 0);
        assert_eq!(chunk_count(100, 1024), 1);
        assert_eq!(chunk_count(1024, 1024), 1);
        assert_eq!(chunk_count(1025, 1024), 2);
    }

    #[test]
    fn test_sha1_hex() {
        let data = b"hello world";
        let hash = sha1_hex(data);
        assert_eq!(hash, "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed");
    }

    #[test]
    fn test_read_write_chunk() -> Result<(), Box<dyn std::error::Error>> {
        let mut temp = NamedTempFile::new()?;
        let chunk_size = CHUNK_SIZE;
        // Crear un archivo del tamaño de 2 chunks
        let total_size = (chunk_size * 2) as u64;
        temp.as_file().set_len(total_size)?;
        let path = temp.path();

        let chunk_data = vec![0xAAu8; chunk_size as usize];
        let hash = sha1_hex(&chunk_data);

        // Escribir en el segundo chunk (índice 1)
        write_chunk_at_offset(path, 1, chunk_size, &chunk_data)?;

        // Leer y verificar
        let read_data = read_chunk(path, 1, chunk_size, &hash)?;
        assert_eq!(read_data.len(), chunk_data.len());
        assert_eq!(read_data, chunk_data);

        Ok(())
    }
}
