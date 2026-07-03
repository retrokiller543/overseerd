//! Transport layer for Overseerd: the [`Transport`]/[`Connection`]/[`Respond`]
//! abstraction, the length-prefixed wire [`protocol`], and concrete transports
//! (TCP, Unix sockets, in-memory).
//!
//! The daemon is generic over [`Transport`]; a transport yields [`Connection`]s,
//! each of which yields `(IncomingCall, Responder)` pairs. Correlation ids live
//! entirely here — the daemon never sees them.
//!
//! Usually consumed through the `overseerd` facade crate.

pub mod codec;
#[cfg(feature = "di")]
mod di;
pub mod error;
pub mod frame;
pub mod protocol;
pub mod status;
pub mod stream_codec;
pub mod transport;
// Concrete socket/in-memory transports drive `tokio::net`/`mio`, unsupported on wasm. The
// `Transport`/`Connection` traits (in `transport`) stay available; only the impls are gated.
#[cfg(not(target_family = "wasm"))]
pub mod transports;

pub use codec::{CodecError, Decodes, Encodes};
pub use error::{Error, Result};
pub use frame::{CallId, CallResult, IncomingCall, PeerInfo};
pub use protocol::{WireMessage, WireOutcome, WireRequest, WireResponse};
pub use status::{Flags, PredefinedCode, StatusCode};
pub use stream_codec::{StreamDecode, StreamDecodeError, StreamEncode, StreamEncodeError};
pub use transport::{Connection, Respond, RespondStream, ResponseSink, Transport};
#[cfg(not(target_family = "wasm"))]
pub use transports::{
    MemoryCall, MemoryClient, MemoryConnection, MemoryConnectionHandle, MemoryResponder,
    MemorySink, MemoryTransport, ServerEvent, StreamConnection, StreamResponder, StreamSink,
    TcpConnection, TcpResponder, TcpTransport,
};

#[cfg(unix)]
pub use transports::{UnixConnection, UnixResponder, UnixTransport};
