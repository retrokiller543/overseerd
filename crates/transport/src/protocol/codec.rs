use std::time::Duration;

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::{Instant, timeout_at},
};

use crate::error::{Error, Result};

use super::WireMessage;

/// Upper bound on a single frame's payload. Guards against a peer sending a
/// huge length prefix that would otherwise trigger an unbounded allocation.
pub const DEFAULT_MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

/// Default time a peer may go without making progress on the current frame.
pub const DEFAULT_READ_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

const READ_CHUNK_LEN: usize = 8 * 1024;
const RETAINED_PAYLOAD_CAPACITY: usize = READ_CHUNK_LEN * 2;

/// Resource limits for length-prefixed message decoding.
///
/// The timeout is an *idle* deadline: it restarts after every successful read,
/// allowing large frames to arrive over slow links as long as they continue to
/// make progress.
#[derive(Clone, Copy, Debug)]
pub struct FrameConfig {
    max_frame_len: usize,
    read_idle_timeout: Duration,
}

impl FrameConfig {
    /// Creates limits with the supplied non-zero maximum frame size and idle
    /// timeout.
    pub fn new(max_frame_len: usize, read_idle_timeout: Duration) -> Self {
        assert!(max_frame_len > 0, "maximum frame length must be non-zero");
        assert!(
            !read_idle_timeout.is_zero(),
            "read idle timeout must be non-zero"
        );

        Self {
            max_frame_len,
            read_idle_timeout,
        }
    }

    pub fn max_frame_len(self) -> usize {
        self.max_frame_len
    }

    pub fn read_idle_timeout(self) -> Duration {
        self.read_idle_timeout
    }
}

impl Default for FrameConfig {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_FRAME_LEN, DEFAULT_READ_IDLE_TIMEOUT)
    }
}

/// A cancellation-safe, incremental decoder for the wire's length-prefixed
/// messages.
///
/// Prefix and payload progress live in this value rather than in the returned
/// future. Dropping an in-progress [`read_message`](Self::read_message) future
/// therefore never discards bytes already consumed from the underlying stream.
pub struct MessageReader<R> {
    reader: R,
    config: FrameConfig,
    prefix: [u8; 4],
    prefix_read: usize,
    expected_len: Option<usize>,
    payload: Vec<u8>,
    /// Absolute idle deadline for the frame currently being read. Keeping it in the reader makes
    /// the deadline survive cancellation by connection-level maintenance branches.
    idle_deadline: Option<Instant>,
}

impl<R> MessageReader<R> {
    pub fn new(reader: R) -> Self {
        Self::with_config(reader, FrameConfig::default())
    }

    pub fn with_config(reader: R, config: FrameConfig) -> Self {
        Self {
            reader,
            config,
            prefix: [0; 4],
            prefix_read: 0,
            expected_len: None,
            payload: Vec::new(),
            idle_deadline: None,
        }
    }

    pub fn config(&self) -> FrameConfig {
        self.config
    }

    pub fn into_inner(self) -> R {
        self.reader
    }
}

impl<R> MessageReader<R>
where
    R: AsyncRead + Unpin,
{
    /// Reads and deserializes one message while retaining partial-frame state
    /// across cancellation.
    pub async fn read_message(&mut self) -> Result<WireMessage> {
        while self.prefix_read < self.prefix.len() {
            let read = if self.prefix_read == 0 {
                // A connection may be idle indefinitely between frames. Read only
                // the first byte without a deadline; once it arrives, the peer has
                // committed to a frame and every subsequent read is idle-timed.
                let read = read_once(&mut self.reader, &mut self.prefix[..1]).await?;
                self.idle_deadline = Some(Instant::now() + self.config.read_idle_timeout);
                read
            } else {
                let read = read_with_idle_deadline(
                    &mut self.reader,
                    &mut self.prefix[self.prefix_read..],
                    self.idle_deadline
                        .expect("a partial frame always has an idle deadline"),
                    self.config.read_idle_timeout,
                )
                .await?;
                self.idle_deadline = Some(Instant::now() + self.config.read_idle_timeout);
                read
            };

            self.prefix_read += read;
        }

        let expected_len = match self.expected_len {
            Some(len) => len,
            None => {
                let len = u32::from_le_bytes(self.prefix) as usize;

                if len > self.config.max_frame_len {
                    return Err(Error::FrameTooLarge {
                        len,
                        max: self.config.max_frame_len,
                    });
                }

                self.expected_len = Some(len);
                len
            }
        };

        let mut chunk = [0_u8; READ_CHUNK_LEN];

        while self.payload.len() < expected_len {
            let remaining = expected_len - self.payload.len();
            let chunk_len = remaining.min(chunk.len());
            let read = read_with_idle_deadline(
                &mut self.reader,
                &mut chunk[..chunk_len],
                self.idle_deadline
                    .expect("a partial frame always has an idle deadline"),
                self.config.read_idle_timeout,
            )
            .await?;
            self.idle_deadline = Some(Instant::now() + self.config.read_idle_timeout);

            self.payload
                .try_reserve(read)
                .map_err(|_| Error::FrameAllocation { len: expected_len })?;
            self.payload.extend_from_slice(&chunk[..read]);
        }

        let decoded =
            postcard::from_bytes(&self.payload).map_err(|e| Error::Serialization(e.to_string()));

        self.prefix = [0; 4];
        self.prefix_read = 0;
        self.expected_len = None;
        self.idle_deadline = None;
        if self.payload.capacity() > RETAINED_PAYLOAD_CAPACITY {
            self.payload = Vec::new();
        } else {
            self.payload.clear();
        }

        decoded
    }
}

async fn read_once<R: AsyncRead + Unpin>(reader: &mut R, buf: &mut [u8]) -> Result<usize> {
    match reader.read(buf).await {
        Ok(0) => Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into()),
        Ok(read) => Ok(read),
        Err(error) => Err(error.into()),
    }
}

async fn read_with_idle_deadline<R: AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut [u8],
    deadline: Instant,
    idle_timeout: Duration,
) -> Result<usize> {
    match timeout_at(deadline, reader.read(buf)).await {
        Ok(Ok(0)) => Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into()),
        Ok(Ok(read)) => Ok(read),
        Ok(Err(error)) => Err(error.into()),
        Err(_) => Err(Error::ReadTimeout { idle_timeout }),
    }
}

/// Reads a length-prefixed frame from a stream and deserializes it.
///
/// Frame layout: `[u32 LE payload length][postcard-encoded WireMessage]`
pub async fn read_message<R: AsyncRead + Unpin>(reader: &mut R) -> Result<WireMessage> {
    MessageReader::new(reader).read_message().await
}

/// Serializes a message and writes it as a length-prefixed frame to a stream.
pub async fn write_message<W: AsyncWrite + Unpin>(writer: &mut W, msg: &WireMessage) -> Result<()> {
    let bytes = postcard::to_allocvec(msg).map_err(|e| Error::Serialization(e.to_string()))?;

    writer.write_u32_le(bytes.len() as u32).await?;
    writer.write_all(&bytes).await?;

    Ok(())
}

#[cfg(test)]
mod tests;
