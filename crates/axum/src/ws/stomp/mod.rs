//! STOMP 1.2 pub/sub over WebSocket.
//!
//! [`Stomp`] is a [`WebsocketProtocol`](crate::ws::WebsocketProtocol) implementation that adds a
//! broker on top of the shared ws seam: a `SEND` to an app destination (`/app/**`) invokes a
//! `#[message]` handler, a `SUBSCRIBE` registers interest, and a `SEND` to a broker destination
//! (`/topic/**`, `/queue/**`) — or an app handler publishing through a [`Publisher`] — fans a
//! `MESSAGE` out to every subscriber, across connections.
//!
//! Framing is delegated to the [`stomp-parser`](https://crates.io/crates/stomp-parser) crate;
//! this module owns the broker, the connection serve loop, DI scope seeding, and the typed
//! [`Topic`]/[`Publisher`] publish surface. Message-body serialization is pluggable per topic set
//! via [`StompCodec`] (`#[topics(codec = ..)]`), defaulting to [`JsonCodec`].

mod body;
mod broker;
mod error;
mod headers;
mod publisher;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use overseerd_app::AppRuntime;
use overseerd_core::TypeDescriptor;
use overseerd_di::{BoxedComponent, ScopeContainer};
use stomp_parser::client::ClientFrame;
use stomp_parser::headers::{StompVersion, StompVersions};
use stomp_parser::server::{ConnectedFrameBuilder, ErrorFrame, ReceiptFrameBuilder};
use tokio::sync::mpsc;

use super::{
    WebsocketProtocol, WsControllerDescriptor, WsDispatchError, WsHandlerFn, WsRespond, WsShutdown,
};
use crate::scope::Request as RequestScope;

pub use body::{JsonCodec, Publish, StompBody, StompCodec, StompOutcome, Topic, TopicParam};
pub use broker::{Broker, ConnectionId};
pub use error::StompError;
pub use headers::{StompHeaders, StompSession};
pub use publisher::Publisher;

use broker::OutFrame;

/// The outbound-frame channel depth per connection before publishes to a slow consumer are dropped.
const OUTBOUND_BUFFER: usize = 64;

/// Heart-beat and version policy for a [`Stomp`] endpoint.
#[derive(Clone)]
pub struct StompConfig {
    /// If set, the server emits a heart-beat this often when otherwise idle (and advertises it in
    /// `CONNECTED`). `None` disables server heart-beating.
    pub server_heartbeat: Option<Duration>,

    /// The protocol versions the server accepts, highest preference first.
    pub versions: Vec<StompVersion>,
}

impl Default for StompConfig {
    fn default() -> Self {
        Self {
            server_heartbeat: None,
            versions: vec![StompVersion::V1_2, StompVersion::V1_1],
        }
    }
}

/// The STOMP protocol: a destination → `#[message]` handler table for `/app/**` plus a shared
/// [`Broker`] for `/topic/**` fan-out.
pub struct Stomp {
    app_routes: HashMap<&'static str, WsHandlerFn<Stomp>>,
    broker: Arc<Broker>,
    runtime: AppRuntime,
    config: StompConfig,
}

impl WebsocketProtocol for Stomp {
    type Payload = StompBody;
    type Outcome = StompOutcome;
    type Options = StompConfig;

    fn build(
        controllers: &[WsControllerDescriptor],
        runtime: &AppRuntime,
        config: StompConfig,
    ) -> Self {
        let mut app_routes: HashMap<&'static str, WsHandlerFn<Stomp>> = HashMap::new();

        for descriptor in controllers {
            for route in descriptor.routes_for::<Stomp>(runtime) {
                if app_routes
                    .insert(route.destination, route.handler)
                    .is_some()
                {
                    tracing::warn!(
                        target: "overseerd::axum",
                        dest = route.destination,
                        "duplicate STOMP destination registered; last registration wins"
                    );
                }
            }
        }

        Self {
            app_routes,
            broker: Arc::new(Broker::new()),
            runtime: runtime.clone(),
            config,
        }
    }

    async fn serve(
        self: Arc<Self>,
        socket: WebSocket,
        connection: Arc<ScopeContainer>,
        mut shutdown: WsShutdown,
    ) {
        let (mut sender, mut receiver) = socket.split();
        let conn_id = self.broker.register();

        // The handshake must come first: read one frame and expect CONNECT/STOMP. A non-CONNECT
        // opener (or a parse failure) is a protocol violation — reply ERROR and abandon the socket.
        let negotiated = match receiver.next().await {
            Some(Ok(message)) => match self.negotiate(message) {
                Ok(version) => version,

                Err(error) => {
                    let _ = sender.send(to_message(error_frame(&error))).await;

                    return;
                }
            },

            _ => return,
        };

        let (tx, rx) = mpsc::channel::<OutFrame>(OUTBOUND_BUFFER);
        let writer = tokio::spawn(writer_task(sender, rx, self.config.server_heartbeat));

        // CONNECTED confirms the negotiated version (and advertises the server heart-beat).
        let _ = tx
            .send(OutFrame::Frame(connected_frame(negotiated, &self.config)))
            .await;

        loop {
            tokio::select! {
                _ = shutdown.wait() => {
                    let _ = tx.send(OutFrame::Frame(error_frame(&StompError::Frame(
                        "server shutting down".to_owned(),
                    )))).await;

                    break;
                }

                inbound = receiver.next() => {
                    match inbound {
                        // A bare newline (or empty frame) is a client heart-beat, not a STOMP frame;
                        // consume it silently rather than failing to parse it (real clients such as
                        // stomp.js send these).
                        Some(Ok(Message::Text(text))) if is_heartbeat(text.as_bytes()) => {}

                        Some(Ok(Message::Text(text))) => {
                            if self.dispatch(text.as_bytes().to_vec(), &tx, conn_id, &connection).await.is_break() {
                                break;
                            }
                        }

                        Some(Ok(Message::Binary(bytes))) if is_heartbeat(&bytes) => {}

                        Some(Ok(Message::Binary(bytes))) => {
                            if self.dispatch(bytes.to_vec(), &tx, conn_id, &connection).await.is_break() {
                                break;
                            }
                        }

                        Some(Ok(Message::Close(_))) | None => break,

                        // Ping/Pong are handled by axum; nothing else is meaningful here.
                        Some(Ok(_)) => {}

                        Some(Err(error)) => {
                            tracing::debug!(target: "overseerd::axum", %error, "STOMP connection read error");

                            break;
                        }
                    }
                }
            }
        }

        self.broker.unregister(conn_id);
        drop(tx);
        let _ = writer.await;
    }
}

impl Stomp {
    /// The shared broker, so app code outside a handler can publish too.
    pub fn broker(&self) -> &Arc<Broker> {
        &self.broker
    }

    /// Negotiates the protocol version from a CONNECT/STOMP frame, erroring on a non-CONNECT opener
    /// or when no offered version is supported.
    fn negotiate(&self, message: Message) -> Result<StompVersion, StompError> {
        let bytes = match message {
            Message::Text(text) => text.as_bytes().to_vec(),
            Message::Binary(bytes) => bytes.to_vec(),

            _ => return Err(StompError::Frame("expected a CONNECT frame".to_owned())),
        };

        // STOMP marks `host` mandatory on CONNECT, but many clients (notably stomp.js) omit it and
        // most brokers tolerate that. `stomp-parser` is strict, so inject a synthetic host when one
        // is absent — `host` is informational and this server ignores it (it reads only the
        // negotiated version).
        let bytes = ensure_connect_host(bytes);

        let frame =
            ClientFrame::try_from(bytes).map_err(|e| StompError::Frame(e.message().to_owned()))?;

        let ClientFrame::Connect(connect) = frame else {
            return Err(StompError::UnexpectedCommand(
                "expected CONNECT before any other frame".to_owned(),
            ));
        };

        let offered: &StompVersions = connect.accept_version().value();

        self.config
            .versions
            .iter()
            .find(|candidate| offered.iter().any(|v| v == *candidate))
            .cloned()
            .ok_or_else(|| StompError::VersionMismatch {
                offered: offered.to_string(),
            })
    }

    /// Parses and routes one inbound frame. Returns [`ControlFlow::Break`] when the connection
    /// should close (DISCONNECT or a fatal protocol error).
    async fn dispatch(
        &self,
        bytes: Vec<u8>,
        tx: &mpsc::Sender<OutFrame>,
        conn_id: ConnectionId,
        connection: &Arc<ScopeContainer>,
    ) -> std::ops::ControlFlow<()> {
        use std::ops::ControlFlow::{Break, Continue};

        let frame = match ClientFrame::try_from(bytes) {
            Ok(frame) => frame,

            Err(error) => {
                let _ = tx
                    .send(OutFrame::Frame(error_frame(&StompError::Frame(
                        error.message().to_owned(),
                    ))))
                    .await;

                return Break(());
            }
        };

        match frame {
            ClientFrame::Subscribe(sub) => {
                let receipt = sub.receipt().map(|r| r.value().to_owned());

                self.broker.subscribe(
                    conn_id,
                    sub.id().value(),
                    sub.destination().value(),
                    tx.clone(),
                );

                self.send_receipt(tx, receipt).await;

                Continue(())
            }

            ClientFrame::Unsubscribe(unsub) => {
                let receipt = unsub.receipt().map(|r| r.value().to_owned());

                self.broker.unsubscribe(conn_id, unsub.id().value());
                self.send_receipt(tx, receipt).await;

                Continue(())
            }

            ClientFrame::Send(send) => {
                let destination = send.destination().value().to_owned();
                let receipt = send.receipt().map(|r| r.value().to_owned());
                let content_type = send.content_type().map(|c| c.value().to_owned());
                let body = StompBody {
                    content_type: content_type.clone(),
                    bytes: send
                        .body()
                        .map(bytes::Bytes::copy_from_slice)
                        .unwrap_or_default(),
                };

                self.route_send(&destination, body, content_type, conn_id, connection)
                    .await;
                self.send_receipt(tx, receipt).await;

                Continue(())
            }

            ClientFrame::Disconnect(disconnect) => {
                let receipt = Some(disconnect.receipt().value().to_owned());

                self.send_receipt(tx, receipt).await;

                Break(())
            }

            other => {
                let _ = tx
                    .send(OutFrame::Frame(error_frame(
                        &StompError::UnexpectedCommand(format!("{other:?}")),
                    )))
                    .await;

                Break(())
            }
        }
    }

    /// Routes a `SEND`: an `/app/**` destination invokes its handler (seeding the message scope with
    /// the frame headers and a session handle); any other destination is a direct broker publish.
    async fn route_send(
        &self,
        destination: &str,
        body: StompBody,
        content_type: Option<String>,
        conn_id: ConnectionId,
        connection: &Arc<ScopeContainer>,
    ) {
        let Some(handler) = self.app_routes.get(destination) else {
            self.broker.publish(destination, &body, &[]);

            return;
        };

        let mut header_list = vec![("destination".to_owned(), destination.to_owned())];

        if let Some(ct) = content_type {
            header_list.push(("content-type".to_owned(), ct));
        }

        let seeds = vec![
            BoxedComponent {
                ty: TypeDescriptor::of::<StompHeaders>("StompHeaders"),
                value: Box::new(StompHeaders::new(header_list)),
            },
            BoxedComponent {
                ty: TypeDescriptor::of::<StompSession>("StompSession"),
                value: Box::new(StompSession::new(Arc::clone(&self.broker), conn_id)),
            },
        ];

        let scope = match self
            .runtime
            .open_scope(&RequestScope, Arc::clone(connection), seeds)
            .await
        {
            Ok(scope) => scope,

            Err(error) => {
                tracing::error!(target: "overseerd::axum", %error, "STOMP message scope build failed");

                return;
            }
        };

        match handler(body, scope).await {
            Ok(StompOutcome::Publish(publishes)) => {
                for publish in publishes {
                    self.broker
                        .publish(&publish.destination, &publish.body, &publish.headers);
                }
            }

            Ok(StompOutcome::Nothing) => {}

            Err(error) => {
                tracing::warn!(target: "overseerd::axum", %error, dest = destination, "STOMP handler failed");
            }
        }
    }

    /// Sends a `RECEIPT` for `receipt_id`, if the client requested one.
    async fn send_receipt(&self, tx: &mpsc::Sender<OutFrame>, receipt_id: Option<String>) {
        if let Some(id) = receipt_id {
            let frame: Vec<u8> = ReceiptFrameBuilder::new(id).build().into();
            let _ = tx.send(OutFrame::Frame(frame)).await;
        }
    }
}

/// What a STOMP `#[message]` handler may return, turned into a [`StompOutcome`]. Keeps
/// [`WsRespond`] a single blanket impl (avoiding overlap) while accepting `()`, an explicit
/// outcome, one or many [`Publish`]es, or a `Result` of any of those.
pub trait IntoStompOutcome {
    /// Converts this handler return value into a protocol outcome.
    fn into_outcome(self) -> Result<StompOutcome, WsDispatchError>;
}

impl IntoStompOutcome for () {
    fn into_outcome(self) -> Result<StompOutcome, WsDispatchError> {
        Ok(StompOutcome::Nothing)
    }
}

impl IntoStompOutcome for StompOutcome {
    fn into_outcome(self) -> Result<StompOutcome, WsDispatchError> {
        Ok(self)
    }
}

impl IntoStompOutcome for Publish {
    fn into_outcome(self) -> Result<StompOutcome, WsDispatchError> {
        Ok(StompOutcome::Publish(vec![self]))
    }
}

impl IntoStompOutcome for Vec<Publish> {
    fn into_outcome(self) -> Result<StompOutcome, WsDispatchError> {
        Ok(StompOutcome::Publish(self))
    }
}

impl<T, E> IntoStompOutcome for Result<T, E>
where
    T: IntoStompOutcome,
    E: std::fmt::Display,
{
    fn into_outcome(self) -> Result<StompOutcome, WsDispatchError> {
        self.map_err(|e| WsDispatchError::Encode(e.to_string()))?
            .into_outcome()
    }
}

impl<R> WsRespond<R> for Stomp
where
    R: IntoStompOutcome,
{
    fn respond(response: R) -> Result<StompOutcome, WsDispatchError> {
        response.into_outcome()
    }
}

/// The per-connection writer task: drains queued frames to the socket and emits server heart-beats
/// when idle. Owns the socket's write half so any connection's publish reaches this socket without
/// touching the reader loop.
async fn writer_task(
    mut sender: futures::stream::SplitSink<WebSocket, Message>,
    mut rx: mpsc::Receiver<OutFrame>,
    heartbeat: Option<Duration>,
) {
    let mut ticker = heartbeat.map(tokio::time::interval);

    if let Some(ticker) = ticker.as_mut() {
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    }

    loop {
        tokio::select! {
            frame = rx.recv() => match frame {
                Some(OutFrame::Frame(bytes)) => {
                    if sender.send(to_message(bytes)).await.is_err() {
                        break;
                    }
                }

                Some(OutFrame::Heartbeat) => {
                    if sender.send(Message::text("\n")).await.is_err() {
                        break;
                    }
                }

                None => break,
            },

            _ = tick(ticker.as_mut()) => {
                if sender.send(Message::text("\n")).await.is_err() {
                    break;
                }
            }
        }
    }
}

/// Resolves on the next heart-beat tick, or never when heart-beating is disabled — so the
/// `select!` arm is inert without a timer.
async fn tick(ticker: Option<&mut tokio::time::Interval>) {
    match ticker {
        Some(ticker) => {
            ticker.tick().await;
        }

        None => std::future::pending::<()>().await,
    }
}

/// Whether an inbound frame is a client heart-beat — an empty payload or a bare newline
/// (`\n` / `\r\n`) — rather than a STOMP frame to parse.
fn is_heartbeat(bytes: &[u8]) -> bool {
    bytes.is_empty() || bytes == b"\n" || bytes == b"\r\n"
}

/// Injects a `host:overseerd` header into a `CONNECT`/`STOMP` frame that lacks one, so a client
/// that omits the (spec-mandatory but widely-skipped) `host` header still connects. Leaves any
/// other frame — and a CONNECT that already has a host — untouched.
fn ensure_connect_host(bytes: Vec<u8>) -> Vec<u8> {
    let is_connect = bytes.starts_with(b"CONNECT") || bytes.starts_with(b"STOMP");

    if !is_connect || contains_subsequence(&bytes, b"\nhost:") {
        return bytes;
    }

    let Some(newline) = bytes.iter().position(|&b| b == b'\n') else {
        return bytes;
    };

    let mut out = Vec::with_capacity(bytes.len() + 16);

    out.extend_from_slice(&bytes[..=newline]);
    out.extend_from_slice(b"host:overseerd\n");
    out.extend_from_slice(&bytes[newline + 1..]);

    out
}

/// Whether `haystack` contains `needle` as a contiguous subsequence.
fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use stomp_parser::client::ClientFrame;

    use super::*;

    #[test]
    fn host_is_injected_so_a_hostless_connect_parses() {
        // A stomp.js-style CONNECT with no `host` header — rejected by stomp-parser as-is.
        let frame = b"CONNECT\naccept-version:1.0,1.1,1.2\nheart-beat:0,0\n\n\x00".to_vec();
        assert!(
            ClientFrame::try_from(frame.clone()).is_err(),
            "hostless CONNECT is rejected raw"
        );

        let patched = ensure_connect_host(frame);
        let parsed = ClientFrame::try_from(patched).expect("patched CONNECT parses");

        assert!(matches!(parsed, ClientFrame::Connect(_)));
    }

    #[test]
    fn a_connect_with_a_host_is_left_untouched() {
        let frame = b"CONNECT\naccept-version:1.2\nhost:example\n\n\x00".to_vec();
        let out = ensure_connect_host(frame.clone());

        assert_eq!(out, frame, "an existing host is not duplicated");
    }

    #[test]
    fn non_connect_frames_are_left_untouched() {
        let frame = b"SEND\ndestination:/app/chat\n\nhi\x00".to_vec();
        let out = ensure_connect_host(frame.clone());

        assert_eq!(out, frame);
    }
}

/// Wraps serialized frame bytes in a WebSocket message: text when valid UTF-8 (the common case for
/// STOMP's text protocol), binary otherwise (a binary body).
fn to_message(bytes: Vec<u8>) -> Message {
    match String::from_utf8(bytes) {
        Ok(text) => Message::text(text),

        Err(error) => Message::binary(error.into_bytes()),
    }
}

/// Serializes a `CONNECTED` frame confirming the negotiated version.
fn connected_frame(version: StompVersion, _config: &StompConfig) -> Vec<u8> {
    ConnectedFrameBuilder::new(version).build().into()
}

/// Serializes an `ERROR` frame carrying the error's message.
fn error_frame(error: &StompError) -> Vec<u8> {
    ErrorFrame::from_message(&error.to_string()).into()
}
