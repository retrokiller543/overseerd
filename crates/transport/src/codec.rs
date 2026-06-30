//! The message-body serialization seam, shared by client and server protocol code.
//!
//! The framework never assumes a serialization format. A protocol declares which message
//! types it can carry by implementing [`Encodes<T>`] / [`Decodes<T>`] for them — typically a
//! blanket `impl<T: Serialize> Encodes<T>` (postcard/JSON), or `impl<T: Archive> Encodes<T>`
//! (rkyv), etc. Capability methods (client calls, server handlers) are bound on these, so a
//! message a protocol cannot serialize is a compile error — protocol-defined, never assumed.
//!
//! Both directions are split (mirroring the per-direction stream codecs) so each use site
//! bounds exactly the one it needs: a client encodes requests and decodes responses; a server
//! does the reverse.

use crate::status::{PredefinedCode, StatusCode};

/// A body encode/decode failure. Carries a [`StatusCode`] (so a server can map it onto an
/// error response — a decode failure is `BadInput`) and a human-readable message.
#[derive(Debug, Clone)]
pub struct CodecError {
    pub code: StatusCode,
    pub message: String,
}

impl CodecError {
    /// A `BadInput`-coded failure — the usual classification for a body that did not
    /// encode/decode.
    pub fn bad_input(message: impl Into<String>) -> Self {
        Self {
            code: StatusCode::from(PredefinedCode::BadInput),
            message: message.into(),
        }
    }

    /// An `Internal`-coded failure, for an encode error that is the local side's fault.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: StatusCode::from(PredefinedCode::Internal),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CodecError {}

/// A protocol can encode a `T` (a request, response, or stream item) into a wire body.
/// Implemented by a protocol for the message types it supports.
pub trait Encodes<T>: Send + Sync {
    fn encode(&self, value: T) -> Result<Vec<u8>, CodecError>;
}

/// A protocol can decode a `T` (a request, response, or stream item) from a wire body.
pub trait Decodes<T>: Send + Sync {
    fn decode(&self, body: Vec<u8>) -> Result<T, CodecError>;
}
