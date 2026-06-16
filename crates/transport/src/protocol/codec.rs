use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{Error, Result};

use super::WireMessage;

/// Upper bound on a single frame's payload. Guards against a peer sending a
/// huge length prefix that would otherwise trigger an unbounded allocation.
const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

/// Reads a length-prefixed frame from a stream and deserializes it.
///
/// Frame layout: `[u32 LE payload length][postcard-encoded WireMessage]`
pub async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> Result<WireMessage> {
    let len = reader.read_u32_le().await? as usize;

    if len > MAX_FRAME_LEN {
        return Err(Error::FrameTooLarge { len, max: MAX_FRAME_LEN });
    }

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
