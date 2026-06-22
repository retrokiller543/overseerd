//! The per-item stream codecs that let a type own its on-the-wire stream format.
//!
//! Streamed items cross the wire through two orthogonal, single-operation traits:
//! [`StreamEncode`] turns an item into a frame (used wherever a side *sends* —
//! the client for request items, the server for response items) and
//! [`StreamDecode`] turns a frame back into an item (used wherever a side
//! *receives*). Splitting by operation rather than direction means each use site
//! bounds exactly the one it needs, and a type implements only the half it is
//! actually used for.
//!
//! [`encode`](StreamEncode::encode)/[`decode`](StreamDecode::decode) define the
//! per-item format (no default; each implementor provides them);
//! [`into_frames`](StreamEncode::into_frames)/[`from_frames`](StreamDecode::from_frames)
//! lift them to whole-stream framing with overridable defaults that map
//! item-by-item, so a type can additionally control batching, headers, or
//! terminal handling.
//!
//! A blanket [`StreamEncode`] covers every `Serialize` type and a blanket
//! [`StreamDecode`] every `DeserializeOwned` type, both via `postcard`, so
//! ordinary items stream with zero boilerplate. A type wanting a custom format
//! implements the relevant trait directly — and is then not `Serialize` /
//! `DeserializeOwned`, so it does not overlap the blanket (the same coherence
//! dance as `Responder`/`ResponseError`).

use std::fmt;

use futures::{Stream, StreamExt};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::status::{PredefinedCode, StatusCode};

/// A failure encoding a stream item into a wire frame. Carries the `StatusCode`
/// and body the terminating `StreamError` frame will report, mirroring the
/// handler-side error currency so a custom codec can classify its own failures.
#[derive(Debug, Clone)]
pub struct StreamEncodeError {
    pub code: StatusCode,
    pub body: Vec<u8>,
}

impl StreamEncodeError {
    /// An `Internal`-coded encode failure with an empty body — the fallback when a
    /// codec has nothing more specific to say.
    pub fn internal() -> Self {
        Self {
            code: StatusCode::from(PredefinedCode::Internal),
            body: Vec::new(),
        }
    }
}

impl fmt::Display for StreamEncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "stream item encode failed (status {:#010x})", self.code.raw())
    }
}

impl std::error::Error for StreamEncodeError {}

/// A failure decoding a wire frame back into a stream item.
#[derive(Debug, Clone)]
pub struct StreamDecodeError(pub String);

impl fmt::Display for StreamDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "stream item decode failed: {}", self.0)
    }
}

impl std::error::Error for StreamDecodeError {}

/// The send half of a stream item's wire format: how an item becomes frame bytes.
///
/// Bound wherever a side produces stream items — the client for request items, the
/// server for response items. Implement [`encode`](Self::encode) for a custom
/// format; override [`into_frames`](Self::into_frames) to control framing.
pub trait StreamEncode {
    /// Encode one item into its wire frame body.
    fn encode(&self) -> Result<Vec<u8>, StreamEncodeError>;

    /// Lift a stream of items into a stream of wire frames. The default encodes
    /// each item independently; override to batch or reframe.
    fn into_frames<S>(items: S) -> impl Stream<Item = Result<Vec<u8>, StreamEncodeError>> + Send
    where
        S: Stream<Item = Self> + Send + 'static,
        Self: Sized + Send + 'static,
    {
        items.map(|item| item.encode())
    }
}

/// The receive half of a stream item's wire format: how frame bytes become an item.
///
/// Bound wherever a side consumes stream items — the server for request items, the
/// client for response items. Implement [`decode`](Self::decode) for a custom
/// format; override [`from_frames`](Self::from_frames) to match a custom
/// `into_frames`.
pub trait StreamDecode: Sized {
    /// Decode one item from a wire frame body.
    fn decode(bytes: &[u8]) -> Result<Self, StreamDecodeError>;

    /// Lift a stream of inbound wire frames into a stream of items. The default
    /// decodes each frame independently; override to match a custom `into_frames`.
    fn from_frames<S>(frames: S) -> impl Stream<Item = Result<Self, StreamDecodeError>> + Send
    where
        S: Stream<Item = Vec<u8>> + Send + 'static,
        Self: Send + 'static,
    {
        frames.map(|bytes| Self::decode(&bytes))
    }
}

impl<T> StreamEncode for T
where
    T: Serialize,
{
    fn encode(&self) -> Result<Vec<u8>, StreamEncodeError> {
        postcard::to_allocvec(self).map_err(|_| StreamEncodeError::internal())
    }
}

impl<T> StreamDecode for T
where
    T: DeserializeOwned,
{
    fn decode(bytes: &[u8]) -> Result<Self, StreamDecodeError> {
        postcard::from_bytes(bytes).map_err(|e| StreamDecodeError(e.to_string()))
    }
}
