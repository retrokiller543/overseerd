mod stream;

pub mod memory;
pub mod tcp;

#[cfg(unix)]
pub mod unix;

pub use memory::{MemoryClient, MemoryConnection, MemoryConnectionHandle, MemoryResponder, MemoryTransport};
pub use stream::{StreamConnection, StreamResponder};
pub use tcp::{TcpConnection, TcpResponder, TcpTransport};

#[cfg(unix)]
pub use unix::{UnixConnection, UnixResponder, UnixTransport};
