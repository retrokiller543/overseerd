//! Client-side WebSocket request/reply support.
//!
//! The transport owns the socket and request correlation; the protocol owns the frame shape.
//! Downstream protocols implement [`WebsocketClientProtocol`] without changing generated
//! `#[message]` clients.

use overseerd_client::ClientError;
use overseerd_transport::{CodecError, Error as TransportError};

#[cfg(feature = "tungstenite")]
mod tungstenite;

#[cfg(feature = "tungstenite")]
pub use tungstenite::*;

/// A protocol-neutral application frame for the correlated WebSocket client actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsClientFrame {
    /// A UTF-8 WebSocket text frame.
    Text(String),

    /// An opaque WebSocket binary frame.
    Binary(Vec<u8>),
}

/// A websocket request/reply protocol. The framework does not assume numeric request ids: each
/// protocol chooses its own correlation key and frame grammar.
pub trait WebsocketClientProtocol: Send + Sync + 'static {
    type Key: Eq + std::hash::Hash + Clone + Send + 'static;
    type Status: std::fmt::Debug + Copy + Send + 'static;
    type Payload;

    fn next_key(counter: u64) -> Self::Key;

    fn encode_call<Req>(
        destination: &str,
        key: &Self::Key,
        payload: Req,
    ) -> Result<WsClientFrame, CodecError>
    where
        Self: WebsocketEncodes<Req>;

    /// Encodes an uncorrelated fire-and-forget message.
    fn encode_send<Req>(destination: &str, payload: Req) -> Result<WsClientFrame, CodecError>
    where
        Self: WebsocketEncodes<Req>;

    fn reply_key(frame: &WsClientFrame) -> Result<Option<Self::Key>, CodecError>;

    fn decode_reply<Resp>(frame: WsClientFrame) -> Result<Resp, ClientError<Self::Status>>
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
        destination: &str,
        payload: Req,
    ) -> impl std::future::Future<Output = Result<Resp, ClientError<P::Status>>> + Send
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
        _destination: &str,
        _payload: Req,
    ) -> Result<Resp, ClientError<P::Status>>
    where
        Req: Send,
        Resp: Send,
    {
        Err(ClientError::Transport(TransportError::Closed))
    }
}
