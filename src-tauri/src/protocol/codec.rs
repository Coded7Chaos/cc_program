//! Framing TCP: [header_len:u32 BE][data_len:u32 BE][header_json_bytes][raw_bytes]

use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Frame too large: {0} bytes")]
    FrameTooLarge(u32),
    #[error("Connection closed")]
    ConnectionClosed,
}

/// Tamaño máximo de header JSON (64 KB)
const MAX_HEADER_SIZE: u32 = 64 * 1024;
/// Tamaño máximo de datos raw (1.5 MB, un chunk de 1 MB + overhead)
const MAX_DATA_SIZE: u32 = 1536 * 1024;

/// Escribe un frame TCP: [header_len:u32][data_len:u32][header_json][raw_data]
pub async fn write_frame<W, H>(
    writer: &mut W,
    header: &H,
    data: &[u8],
) -> Result<(), CodecError>
where
    W: AsyncWrite + Unpin,
    H: Serialize,
{
    let header_json = serde_json::to_vec(header)?;
    let header_len = header_json.len() as u32;
    let data_len = data.len() as u32;

    let mut buf = BytesMut::with_capacity(8 + header_json.len() + data.len());
    buf.put_u32(header_len);
    buf.put_u32(data_len);
    buf.extend_from_slice(&header_json);
    buf.extend_from_slice(data);

    writer.write_all(&buf).await?;
    writer.flush().await?;
    Ok(())
}

/// Lee un frame TCP y retorna (header_json_bytes, data_bytes)
pub async fn read_frame<R>(
    reader: &mut R,
) -> Result<(Bytes, Bytes), CodecError>
where
    R: AsyncRead + Unpin,
{
    let mut prefix = [0u8; 8];
    match reader.read_exact(&mut prefix).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(CodecError::ConnectionClosed);
        }
        Err(e) => return Err(CodecError::Io(e)),
    }

    let header_len = u32::from_be_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]);
    let data_len = u32::from_be_bytes([prefix[4], prefix[5], prefix[6], prefix[7]]);

    if header_len > MAX_HEADER_SIZE {
        return Err(CodecError::FrameTooLarge(header_len));
    }
    if data_len > MAX_DATA_SIZE {
        return Err(CodecError::FrameTooLarge(data_len));
    }

    let mut header_buf = vec![0u8; header_len as usize];
    reader.read_exact(&mut header_buf).await?;

    let mut data_buf = vec![0u8; data_len as usize];
    if data_len > 0 {
        reader.read_exact(&mut data_buf).await?;
    }

    Ok((Bytes::from(header_buf), Bytes::from(data_buf)))
}

/// Lee un frame y deserializa el header JSON
pub async fn read_typed_frame<R, H>(
    reader: &mut R,
) -> Result<(H, Bytes), CodecError>
where
    R: AsyncRead + Unpin,
    H: DeserializeOwned,
{
    let (header_bytes, data_bytes) = read_frame(reader).await?;
    let header: H = serde_json::from_slice(&header_bytes)?;
    Ok((header, data_bytes))
}
