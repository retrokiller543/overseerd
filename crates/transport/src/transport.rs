use std::future::Future;

use crate::{
    error::Result,
    frame::{CallResult, IncomingCall, PeerInfo},
};

/// Sends the response back to the caller of a specific inbound RPC.
///
/// A responder is created 1:1 with its call and privately owns that call's
/// wire correlation id, so the daemon supplies only the outcome — it never
/// sees or sets the id.
pub trait Respond {
    fn respond(self, outcome: CallResult) -> impl Future<Output = Result<()>> + Send;
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
