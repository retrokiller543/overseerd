mod stream;

pub mod memory;
pub mod tcp;

#[cfg(unix)]
pub mod unix;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod client_stream;

pub use memory::{
    MemoryCall, MemoryClient, MemoryConnection, MemoryConnectionHandle, MemoryResponder,
    MemorySink, MemoryTransport, ServerEvent,
};
pub use stream::{StreamConnection, StreamResponder, StreamSink};
pub use tcp::{TcpConnection, TcpResponder, TcpTransport};

#[cfg(unix)]
pub use unix::{UnixConnection, UnixResponder, UnixTransport};

#[cfg(feature = "client")]
pub use client::{
    BidiStream, ClientCall, ClientConnection, ClientError, ClientTransport, ClientUpstream,
    ErrorBody, Raw, Reply, ServerStream,
};
#[cfg(feature = "client")]
pub use client_stream::{StreamCall, StreamClientTransport};
