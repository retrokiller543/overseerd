use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{Error, Result};

use super::WireMessage;

/// Reads a length-prefixed frame from a stream and deserializes it.
///
/// Frame layout: `[u32 LE payload length][postcard-encoded WireMessage]`
pub async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> Result<WireMessage> {
    let len = reader.read_u32_le().await? as usize;
    let mut buf = vec![0u8; len];

    reader.read_exact(&mut buf).await?;

    postcard::from_bytes(&buf).map_err(|e| Error::Serialization(e.to_string()))
}

/// Serializes a message and writes it as a length-prefixed frame to a stream.
pub async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    msg: &WireMessage,
) -> Result<()> {
    let bytes = postcard::to_allocvec(msg).map_err(|e| Error::Serialization(e.to_string()))?;

    writer.write_u32_le(bytes.len() as u32).await?;
    writer.write_all(&bytes).await?;

    Ok(())
}

/// Encodes a message to bytes for datagram transports.
pub fn encode(msg: &WireMessage) -> Result<Vec<u8>> {
    postcard::to_allocvec(msg).map_err(|e| Error::Serialization(e.to_string()))
}

/// Decodes a message from raw datagram bytes.
pub fn decode(bytes: &[u8]) -> Result<WireMessage> {
    postcard::from_bytes(bytes).map_err(|e| Error::Serialization(e.to_string()))
}
