//! Transport layer for Overseer: the [`Transport`]/[`Connection`]/[`Respond`]
//! abstraction, the length-prefixed wire [`protocol`], and concrete transports
//! (TCP, Unix sockets, in-memory).
//!
//! The daemon is generic over [`Transport`]; a transport yields [`Connection`]s,
//! each of which yields `(IncomingCall, Responder)` pairs. Correlation ids live
//! entirely here — the daemon never sees them.
//!
//! Usually consumed through the `overseer` facade crate.

pub mod error;
pub mod frame;
pub mod protocol;
pub mod status;
pub mod transport;
pub mod transports;

pub use error::{Error, Result};
pub use frame::{CallId, CallResult, IncomingCall, PeerInfo};
pub use protocol::{WireMessage, WireOutcome, WireRequest, WireResponse};
pub use status::{Flags, PredefinedCode, StatusCode};
pub use transport::{Connection, Respond, RespondStream, ResponseSink, Transport};
pub use transports::{
    MemoryCall, MemoryClient, MemoryConnection, MemoryConnectionHandle, MemoryResponder,
    MemorySink, MemoryTransport, ServerEvent, StreamConnection, StreamResponder, StreamSink,
    TcpConnection, TcpResponder, TcpTransport,
};

#[cfg(unix)]
pub use transports::{UnixConnection, UnixResponder, UnixTransport};

#[cfg(feature = "client")]
pub use transports::{
    BidiStream, ClientCall, ClientConnection, ClientError, ClientTransport, ClientUpstream,
    ErrorBody, Raw, Reply, ServerStream, StreamCall, StreamClientTransport,
};
