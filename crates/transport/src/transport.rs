use std::future::Future;

use crate::{
    error::Result,
    frame::{CallResult, IncomingCall, PeerInfo},
};

/// Sends the single response back to the caller of a unary inbound RPC.
///
/// A responder is created 1:1 with its call and privately owns that call's
/// wire correlation id, so the daemon supplies only the outcome — it never
/// sees or sets the id.
pub trait Respond {
    fn respond(self, outcome: CallResult) -> impl Future<Output = Result<()>> + Send;
}

/// Converts a responder into a multi-item sink for server- and bidirectional-
/// streaming calls. Consuming the responder this way replaces the single
/// `respond` with a stream of items terminated by `finish` or `error`.
pub trait RespondStream {
    type Sink: ResponseSink;

    fn into_sink(self) -> Self::Sink;
}

/// The outbound half of a streaming call: many items followed by exactly one
/// terminator. `send` awaits when the peer is slow, providing backpressure.
pub trait ResponseSink: Send {
    /// Sends one response item (a `StreamItem` frame).
    fn send(&mut self, item: Vec<u8>) -> impl Future<Output = Result<()>> + Send;

    /// Terminates the stream with a failure (a `StreamError` frame).
    fn error(self, message: String) -> impl Future<Output = Result<()>> + Send;

    /// Terminates the stream successfully (a `StreamEnd` frame).
    fn finish(self) -> impl Future<Output = Result<()>> + Send;
}

/// A live session between the daemon and one remote peer.
///
/// `recv` yields each new inbound call. For streaming transports the connection
/// also routes a call's subsequent inbound frames to that call internally, so
/// concurrent streams multiplex over one connection and `recv` only surfaces
/// the opening frames.
pub trait Connection: Send + 'static {
    type Responder: Respond + RespondStream + Send + 'static;

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
