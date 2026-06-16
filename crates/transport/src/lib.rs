pub mod error;
pub mod frame;
pub mod protocol;
pub mod transport;
pub mod transports;

pub use error::{Error, Result};
pub use frame::{CallId, CallResult, IncomingCall, PeerInfo};
pub use protocol::{WireMessage, WireOutcome, WireRequest, WireResponse};
pub use transport::{Connection, Respond, Transport};
pub use transports::{
    MemoryClient, MemoryConnection, MemoryConnectionHandle, MemoryResponder, MemoryTransport,
    StreamConnection, StreamResponder, TcpConnection, TcpResponder, TcpTransport,
};

#[cfg(unix)]
pub use transports::{UnixConnection, UnixResponder, UnixTransport};
