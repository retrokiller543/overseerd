//! Client-side WebSocket request/reply support.
//!
//! The transport owns the socket and request correlation; the protocol owns the frame shape.
//! `JsonWs` is the first protocol implementation, and future protocols can implement
//! [`WebsocketClientProtocol`] without changing generated `#[message]` clients.

use overseerd_client::ClientError;
use overseerd_transport::{CodecError, Error as TransportError};

#[cfg(feature = "tungstenite")]
mod tungstenite;

#[cfg(feature = "tungstenite")]
pub use tungstenite::*;

/// WebSocket request/reply status carried by [`ClientError::Remote`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsStatus {
    /// The peer returned an application/protocol error frame.
    Error,
}

/// A websocket request/reply protocol. The framework does not assume numeric request ids: each
/// protocol chooses its own correlation key and frame grammar.
pub trait WebsocketClientProtocol: Send + Sync + 'static {
    type Key: Eq + std::hash::Hash + Clone + Send + 'static;
    type Frame: Send + 'static;
    type Payload;

    fn next_key(counter: u64) -> Self::Key;

    fn encode_call<Req>(
        destination: &str,
        key: &Self::Key,
        payload: Req,
    ) -> Result<Self::Frame, CodecError>
    where
        Self: WebsocketEncodes<Req>;

    fn reply_key(frame: &Self::Frame) -> Result<Option<Self::Key>, CodecError>;

    fn decode_reply<Resp>(frame: Self::Frame) -> Result<Resp, ClientError<WsStatus>>
    where
        Self: WebsocketDecodes<Resp>;
}

/// Encodes a typed websocket message payload for protocol `Self`.
pub trait WebsocketEncodes<T>: WebsocketClientProtocol {
    fn encode_payload(value: T) -> Result<<Self as WebsocketClientProtocol>::Payload, CodecError>;
}

/// Decodes a typed websocket response payload for protocol `Self`.
pub trait WebsocketDecodes<T>: WebsocketClientProtocol {
    fn decode_payload(value: <Self as WebsocketClientProtocol>::Payload) -> Result<T, CodecError>;
}

/// A transport that can issue one typed request/reply shape over websocket protocol `P`.
pub trait WebsocketClient<P, Req, Resp>: Send + Sync
where
    P: WebsocketClientProtocol,
{
    fn websocket_call(
        &self,
        destination: &'static str,
        payload: Req,
    ) -> impl std::future::Future<Output = Result<Resp, ClientError<WsStatus>>> + Send
    where
        Req: Send,
        Resp: Send;
}

impl<P, Req, Resp> WebsocketClient<P, Req, Resp> for ()
where
    P: WebsocketClientProtocol,
{
    async fn websocket_call(
        &self,
        _destination: &'static str,
        _payload: Req,
    ) -> Result<Resp, ClientError<WsStatus>>
    where
        Req: Send,
        Resp: Send,
    {
        Err(ClientError::Transport(TransportError::Closed))
    }
}
