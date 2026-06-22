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
    BidiResponses, CallSink, CallSource, ClientCall, ClientConnection, ClientError,
    ClientTransport, ErrorBody, Raw, Reply, ServerStream, StreamArg,
};
#[cfg(feature = "client")]
pub use client_stream::{StreamCall, StreamCallSink, StreamClientTransport, StreamSource};
