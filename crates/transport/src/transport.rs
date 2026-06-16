use std::future::Future;

use crate::{
    error::Result,
    frame::{IncomingCall, OutgoingResponse, PeerInfo},
};

/// Sends a response back to the caller of a specific inbound RPC.
pub trait Respond {
    fn respond(self, response: OutgoingResponse) -> impl Future<Output = Result<()>> + Send;
}

/// A live session between the daemon and one remote peer.
///
/// Calls on a connection are yielded sequentially. For transports that support
/// parallel streams per connection (e.g. QUIC), each stream maps to one call,
/// and the implementation yields them as they arrive.
pub trait Connection: Send + 'static {
    type Responder: Respond + Send + 'static;

    fn peer(&self) -> &PeerInfo;

    fn recv(
        &mut self,
    ) -> impl Future<Output = Result<Option<(IncomingCall, Self::Responder)>>> + Send;
}

/// An abstract source of inbound connections.
///
/// The daemon is generic over `T: Transport` — chosen at compile time.
/// For multiple transports, implement `Transport` on an enum.
pub trait Transport: Send + 'static {
    type Connection: Connection;

    fn accept(&mut self) -> impl Future<Output = Result<Self::Connection>> + Send;
}
