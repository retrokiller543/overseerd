//! The STOMP client: typed `send`/`subscribe` over a hand-written transport actor.
//!
//! Unlike the request/reply [`WebsocketClientProtocol`](super::WebsocketClientProtocol) (one reply
//! per request), STOMP has three inbound routing lifetimes: `MESSAGE` frames are *durable*, keyed
//! by a client-chosen subscription id; `RECEIPT` frames are *terminal*, keyed by a receipt id; and
//! an `ERROR` frame is *fatal* and connection-terminating. [`StompClientTransport`] runs a
//! background actor demuxing those (modelled on the RPC client's `CallId` demux), and exposes typed
//! [`StompSend`]/[`StompSubscribe`] capabilities the generated clients bind on.

use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;
use overseerd_client::ClientError;
use overseerd_transport::CodecError;

use crate::ws::stomp::StompBody;

/// The status carried by a STOMP [`ClientError::Remote`], mirroring
/// [`WsStatus`](super::WsStatus).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StompStatus {
    /// The broker sent an `ERROR` frame (fatal — the connection is torn down).
    Error,

    /// A protocol violation (e.g. no `CONNECTED` during the handshake, or a version mismatch).
    Protocol,
}

/// A client-chosen subscription id (the routing key for a durable `MESSAGE` stream).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubscriptionId(pub String);

/// A client-chosen receipt id (the routing key for a terminal `RECEIPT`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReceiptId(pub String);

/// A transport that can `SEND` one typed payload to a destination (fire-and-forget). The generated
/// `send_<name>()` client methods bind on this.
pub trait StompSend<Req>: Send + Sync {
    /// Encodes `payload` and writes a `SEND` frame to `destination`.
    fn stomp_send(
        &self,
        destination: &'static str,
        payload: Req,
    ) -> impl std::future::Future<Output = Result<(), ClientError<StompStatus>>> + Send
    where
        Req: Send;
}

/// A transport that can `SUBSCRIBE` to a destination and yield a decoded stream of `MESSAGE`s. The
/// generated `subscribe_<topic>()` client methods bind on this; `decode` is the topic set's codec.
pub trait StompSubscribe: Send + Sync {
    /// Registers a subscription and returns a [`Subscription`] streaming decoded messages.
    fn stomp_subscribe<M>(
        &self,
        destination: &'static str,
        decode: fn(StompBody) -> Result<M, CodecError>,
    ) -> impl std::future::Future<Output = Result<Subscription<Self, M>, ClientError<StompStatus>>> + Send
    where
        Self: Sized + Clone,
        M: Send + 'static;

    /// Deregisters a subscription (best-effort `UNSUBSCRIBE`); called by [`Subscription`]'s `Drop`.
    fn unsubscribe(&self, id: SubscriptionId);
}

/// A live subscription: a [`Stream`] of decoded topic messages that sends `UNSUBSCRIBE` on drop.
/// Returned by every generated `subscribe_<topic>()` method, typed to the topic's message.
pub struct Subscription<C: StompSubscribe, M> {
    id: SubscriptionId,
    items: tokio::sync::mpsc::Receiver<StompBody>,
    decode: fn(StompBody) -> Result<M, CodecError>,
    transport: C,
    _marker: PhantomData<fn() -> M>,
}

impl<C: StompSubscribe, M> Subscription<C, M> {
    /// Assembles a subscription handle. Called by the transport once the `SUBSCRIBE` is registered.
    pub(crate) fn new(
        id: SubscriptionId,
        items: tokio::sync::mpsc::Receiver<StompBody>,
        decode: fn(StompBody) -> Result<M, CodecError>,
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

impl<C, M> Stream for Subscription<C, M>
where
    C: StompSubscribe + Unpin,
    M: Unpin,
{
    type Item = Result<M, ClientError<StompStatus>>;

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

impl<C: StompSubscribe, M> Drop for Subscription<C, M> {
    fn drop(&mut self) {
        self.transport.unsubscribe(self.id.clone());
    }
}

/// The always-closed `()` transport: lets a generated client type-check without a wired transport
/// (mirrors the `()` impls for the request/reply websocket client).
impl<Req> StompSend<Req> for () {
    async fn stomp_send(&self, _: &'static str, _: Req) -> Result<(), ClientError<StompStatus>>
    where
        Req: Send,
    {
        Err(ClientError::Transport(overseerd_transport::Error::Closed))
    }
}

impl StompSubscribe for () {
    async fn stomp_subscribe<M>(
        &self,
        _: &'static str,
        _: fn(StompBody) -> Result<M, CodecError>,
    ) -> Result<Subscription<Self, M>, ClientError<StompStatus>>
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
pub use transport::StompClientTransport;
