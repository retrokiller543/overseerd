//! The pub/sub client capabilities, generic over the protocol: point-to-point *messages* and topic
//! *subscription*.
//!
//! Two message capabilities back the generated `#[message]` controller-client methods:
//! [`MessageSend<P>`] (fire-and-forget) and [`MessageRequest<P>`] (send-and-await-a-reply). The
//! topic capability [`TopicSubscribe<P>`] backs the `#[topics]` `subscribe_*` client — a *durable*
//! [`Subscription`] stream keyed by a client-chosen id, unlike the request/reply
//! [`WebsocketClientProtocol`](super::WebsocketClientProtocol) (one reply per request). A transport
//! (STOMP's [`StompClientTransport`], or another protocol's) implements them for its protocol tag.
//!
//! STOMP's transport sends no heart-beats in v1 (see `docs/stomp.md` for the deferred-feature list).

use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;
use overseerd_client::ClientError;
use overseerd_transport::CodecError;

use crate::stomp::{MessagingClientProtocol, Stomp};

/// The status carried by a STOMP [`ClientError::Remote`], mirroring
/// [`WsStatus`](super::WsStatus). This is [`Stomp`]'s [`MessagingClientProtocol::Status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StompStatus {
    /// The broker sent an `ERROR` frame (fatal — the connection is torn down).
    Error,

    /// A protocol violation (e.g. no `CONNECTED` during the handshake, or a version mismatch).
    Protocol,

    /// A request/response handler on the server returned an error. Non-fatal: only this one call
    /// resolves `Err`, and the connection stays open for further messages.
    Handler,
}

impl MessagingClientProtocol for Stomp {
    type Status = StompStatus;
}

/// A client-chosen subscription id (the routing key for a durable inbound-message stream).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubscriptionId(pub String);

/// A client-chosen receipt id (the routing key for a terminal receipt).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReceiptId(pub String);

/// A transport that can send one typed payload to a destination (fire-and-forget) over protocol `P`.
/// The generated `send_<name>()` client methods bind on this.
pub trait MessageSend<P: MessagingClientProtocol>: Send + Sync {
    /// Writes a send frame carrying `body` to `destination`. The body is already encoded (by the
    /// generated method's codec), so the transport is codec-agnostic — it ships the protocol body.
    /// Takes `&str` (not `&'static str`) so a templated destination works too.
    fn send(
        &self,
        destination: &str,
        body: P::Body,
    ) -> impl std::future::Future<Output = Result<(), ClientError<P::Status>>> + Send;
}

/// A transport that can send one typed payload to a destination and await a correlated typed reply
/// over protocol `P` — the point-to-point request/response companion to [`MessageSend`]. The
/// generated `<name>()` client methods for a non-unit-returning `#[message]` handler bind on this.
pub trait MessageRequest<P: MessagingClientProtocol>: Send + Sync {
    /// Sends `body` to `destination` and resolves with the raw reply body. The request body is
    /// already encoded and the reply is decoded by the generated method's codec, so the transport is
    /// codec-agnostic. Takes `&str` so a templated destination works too.
    fn request(
        &self,
        destination: &str,
        body: P::Body,
    ) -> impl std::future::Future<Output = Result<P::Body, ClientError<P::Status>>> + Send;
}

/// A transport that can subscribe to a destination and yield a decoded stream of messages over
/// protocol `P`. The generated `subscribe_<topic>()` client methods bind on this; `decode` is the
/// topic set's codec.
pub trait TopicSubscribe<P: MessagingClientProtocol>: Send + Sync {
    /// Registers a subscription and returns a [`Subscription`] streaming decoded messages. Takes
    /// `&str` so a templated topic's runtime-rendered destination works too.
    fn subscribe<M>(
        &self,
        destination: &str,
        decode: fn(P::Body) -> Result<M, CodecError>,
    ) -> impl std::future::Future<Output = Result<Subscription<P, Self, M>, ClientError<P::Status>>> + Send
    where
        Self: Sized + Clone,
        M: Send + 'static;

    /// Deregisters a subscription (best-effort); called by [`Subscription`]'s `Drop`.
    fn unsubscribe(&self, id: SubscriptionId);
}

/// A live subscription: a [`Stream`] of decoded topic messages that deregisters on drop. Returned by
/// every generated `subscribe_<topic>()` method, typed to the topic's message and protocol `P`.
pub struct Subscription<P: MessagingClientProtocol, C: TopicSubscribe<P>, M> {
    id: SubscriptionId,
    items: tokio::sync::mpsc::Receiver<P::Body>,
    decode: fn(P::Body) -> Result<M, CodecError>,
    transport: C,
    _marker: PhantomData<fn() -> M>,
}

impl<P: MessagingClientProtocol, C: TopicSubscribe<P>, M> Subscription<P, C, M> {
    /// Assembles a subscription handle. Called by the transport once the subscribe is registered.
    pub(crate) fn new(
        id: SubscriptionId,
        items: tokio::sync::mpsc::Receiver<P::Body>,
        decode: fn(P::Body) -> Result<M, CodecError>,
        transport: C,
    ) -> Self {
        Self {
            id,
            items,
            decode,
            transport,
            _marker: PhantomData,
        }
    }

    /// The subscription id the broker delivers on.
    pub fn id(&self) -> &SubscriptionId {
        &self.id
    }
}

impl<P, C, M> Stream for Subscription<P, C, M>
where
    P: MessagingClientProtocol,
    C: TopicSubscribe<P> + Unpin,
    M: Unpin,
{
    type Item = Result<M, ClientError<P::Status>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        match this.items.poll_recv(cx) {
            Poll::Ready(Some(body)) => {
                let decoded = (this.decode)(body).map_err(|e| ClientError::Decode(e.to_string()));

                Poll::Ready(Some(decoded))
            }

            Poll::Ready(None) => Poll::Ready(None),

            Poll::Pending => Poll::Pending,
        }
    }
}

impl<P: MessagingClientProtocol, C: TopicSubscribe<P>, M> Drop for Subscription<P, C, M> {
    fn drop(&mut self) {
        self.transport.unsubscribe(self.id.clone());
    }
}

/// The always-closed `()` transport: lets a generated client type-check without a wired transport
/// (mirrors the `()` impls for the request/reply websocket client).
impl<P: MessagingClientProtocol> MessageSend<P> for () {
    async fn send(&self, _: &str, _: P::Body) -> Result<(), ClientError<P::Status>> {
        Err(ClientError::Transport(overseerd_transport::Error::Closed))
    }
}

impl<P: MessagingClientProtocol> MessageRequest<P> for () {
    async fn request(&self, _: &str, _: P::Body) -> Result<P::Body, ClientError<P::Status>> {
        Err(ClientError::Transport(overseerd_transport::Error::Closed))
    }
}

impl<P: MessagingClientProtocol> TopicSubscribe<P> for () {
    async fn subscribe<M>(
        &self,
        _: &str,
        _: fn(P::Body) -> Result<M, CodecError>,
    ) -> Result<Subscription<P, Self, M>, ClientError<P::Status>>
    where
        M: Send + 'static,
    {
        Err(ClientError::Transport(overseerd_transport::Error::Closed))
    }

    fn unsubscribe(&self, _: SubscriptionId) {}
}

#[cfg(feature = "tungstenite")]
mod transport;

#[cfg(feature = "tungstenite")]
pub use transport::{StompClientTransport, StompConnectOptions};

// The wasm/JS subscription bridge (the protocol-agnostic pump + handle, and STOMP's
// `TopicWasmClient` impl). wasm-only; the generated `subscribe_*` bindings and the `#[topics]` macro
// name `TopicWasmClient`, `TopicSubscription`, and `pump`.
#[cfg(all(target_family = "wasm", feature = "reqwest", feature = "tungstenite"))]
mod wasm;

#[cfg(all(target_family = "wasm", feature = "reqwest", feature = "tungstenite"))]
pub use wasm::{TopicSubscription, TopicWasmClient, pump};
