//! The tokio-tungstenite STOMP transport actor.
//!
//! [`StompClientTransport::connect`] performs the CONNECT/CONNECTED handshake, then spawns a
//! background task that owns the socket and demuxes inbound frames into three routing tables:
//! subscription id → durable `MESSAGE` stream, receipt id → terminal `RECEIPT`, and a fatal
//! `ERROR`/close that fails everything outstanding (the direct analogue of the RPC client's read
//! loop clearing its call table on disconnect).

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use overseerd_client::{ClientError, ErrorBody};
use overseerd_transport::{CodecError, Error as TransportError};
use stomp_parser::client::{
    ConnectFrameBuilder, DisconnectFrameBuilder, SendFrameBuilder, SubscribeFrameBuilder,
    UnsubscribeFrameBuilder,
};
use stomp_parser::headers::{HeaderValue, StompVersion, StompVersions};
use stomp_parser::server::ServerFrame;
use tokio::sync::{mpsc, oneshot};
// One unified WebSocket type across native and wasm — the socket naming (`MaybeTlsStream<TcpStream>`
// on native, the JS `WebSocket` on wasm) is hidden, so this transport is target-agnostic.
use tokio_tungstenite_wasm::{Error as WsError, Message, WebSocketStream};

use super::StompStatus;
use crate::{MESSAGE_ERROR_HEADER, REPLY_SUBSCRIPTION_ID, Stomp, StompBody};
use overseerd_axum::client::{
    MessageRequest, MessageSend, Subscription, SubscriptionId, TopicSubscribe,
};

/// The write and read halves of a connected WebSocket, split for the actor loop.
type WsWrite = SplitSink<WebSocketStream, Message>;
type WsRead = SplitStream<WebSocketStream>;

/// The outbound-frame and inbound-message channel depths.
const CHANNEL_DEPTH: usize = 64;

/// An acknowledgement that a queued command reached (or failed to reach) the socket.
type Ack = oneshot::Sender<Result<(), ClientError<StompStatus>>>;

/// The delivery of a request's correlated reply body (or the connection error that ended it).
type ReplyTx = oneshot::Sender<Result<StompBody, ClientError<StompStatus>>>;

/// The shared request-correlation table: `correlation-id` → the awaiting reply channel. Shared
/// between the client handles and the actor (rather than actor-owned) so a caller registers its
/// entry synchronously *before* sending — a reply can never race ahead of registration — and an
/// abandoned call removes its own slot directly under the lock. That removes the two failure modes a
/// separate cleanup channel had: no ordering race between a request and its cancellation, and no
/// unbounded cancel queue that a stalled actor could let grow. The lock is only ever held for an
/// infallible map mutation, never across an `.await`.
type Requests = Arc<Mutex<HashMap<String, ReplyTx>>>;

/// Locks the request table, recovering from a poisoned lock: the only code holding it does infallible
/// map mutation, so a guard poisoned by an unrelated panic still protects a consistent map.
fn lock_requests(requests: &Requests) -> MutexGuard<'_, HashMap<String, ReplyTx>> {
    requests.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A command from a client handle to the connection actor.
enum Command {
    Send {
        frame: Vec<u8>,
        ack: Ack,
    },
    /// A request: write the `SEND`. The reply channel is already registered in the shared
    /// [`Requests`] table by the caller (keyed by `correlation-id`) before this is sent, so an
    /// inbound `MESSAGE` can never race ahead of registration. `correlation_id` is carried only so a
    /// write failure can fail the pending entry.
    Request {
        frame: Vec<u8>,
        correlation_id: String,
    },
    Subscribe {
        id: SubscriptionId,
        frame: Vec<u8>,
        items: mpsc::Sender<StompBody>,
        ack: Ack,
    },
    Unsubscribe {
        id: SubscriptionId,
        frame: Vec<u8>,
    },
    /// Sent when the last client handle drops: write a `DISCONNECT` and close gracefully.
    Disconnect {
        frame: Vec<u8>,
        ack: Option<Ack>,
    },
}

/// The shared inner state of a [`StompClientTransport`], behind an `Arc`. Its [`Drop`] fires only
/// when the last client handle is gone — that is when we gracefully `DISCONNECT`.
struct TransportInner {
    tx: mpsc::Sender<Command>,

    /// The shared request-correlation table (see [`Requests`]). Handles register and abandon their
    /// own entries here directly; the actor routes replies and fails the table on close.
    requests: Requests,

    next_id: AtomicU64,

    /// The reply timeout applied to every `request` on this connection (from
    /// [`StompConnectOptions`]).
    request_timeout: Option<Duration>,
}

impl Drop for TransportInner {
    fn drop(&mut self) {
        // Last handle gone: queue a DISCONNECT for the actor to write before the channel closes.
        // Best-effort — a already-closed connection needs no goodbye. The frame is queued on `tx`
        // just before it drops, so the actor drains it, writes DISCONNECT, then sees the channel end.
        let frame: Vec<u8> = DisconnectFrameBuilder::new("bye".to_owned()).build().into();
        let _ = self.tx.try_send(Command::Disconnect { frame, ack: None });
    }
}

/// The default [`request`](MessageRequest::request) reply timeout when
/// [`StompConnectOptions`] does not override it.
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Authentication and custom headers sent on the STOMP `CONNECT` frame, plus the request/response
/// reply timeout.
#[cfg_attr(target_family = "wasm", ::wasm_bindgen::prelude::wasm_bindgen)]
#[derive(Clone)]
pub struct StompConnectOptions {
    host: Option<String>,
    login: Option<String>,
    passcode: Option<String>,
    headers: Vec<(String, String)>,

    /// How long a `request` awaits its correlated reply before abandoning the call with
    /// [`ClientError::Timeout`]. `None` waits indefinitely (until the connection closes). Defaults to
    /// [`DEFAULT_REQUEST_TIMEOUT`]. Enforced on both native (tokio timer) and wasm (browser
    /// `setTimeout`), and it bounds the whole call — command enqueueing plus the reply wait.
    request_timeout: Option<Duration>,
}

impl Default for StompConnectOptions {
    fn default() -> Self {
        Self {
            host: None,
            login: None,
            passcode: None,
            headers: Vec::new(),
            request_timeout: Some(DEFAULT_REQUEST_TIMEOUT),
        }
    }
}

impl fmt::Debug for StompConnectOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let header_names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();

        f.debug_struct("StompConnectOptions")
            .field("host", &self.host)
            .field("login", &self.login)
            .field("passcode", &self.passcode.as_ref().map(|_| "[REDACTED]"))
            .field("header_names", &header_names)
            .finish()
    }
}

#[cfg_attr(target_family = "wasm", ::wasm_bindgen::prelude::wasm_bindgen)]
impl StompConnectOptions {
    /// Empty CONNECT options (`host` defaults to `localhost`).
    #[cfg_attr(
        target_family = "wasm",
        ::wasm_bindgen::prelude::wasm_bindgen(constructor)
    )]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the STOMP virtual host.
    #[cfg_attr(
        target_family = "wasm",
        ::wasm_bindgen::prelude::wasm_bindgen(js_name = setHost)
    )]
    pub fn set_host(&mut self, host: String) {
        self.host = Some(host);
    }

    /// Sets the standard STOMP login credential.
    #[cfg_attr(
        target_family = "wasm",
        ::wasm_bindgen::prelude::wasm_bindgen(js_name = setLogin)
    )]
    pub fn set_login(&mut self, login: String) {
        self.login = Some(login);
    }

    /// Sets the standard STOMP passcode credential.
    #[cfg_attr(
        target_family = "wasm",
        ::wasm_bindgen::prelude::wasm_bindgen(js_name = setPasscode)
    )]
    pub fn set_passcode(&mut self, passcode: String) {
        self.passcode = Some(passcode);
    }

    /// Adds an application-specific CONNECT header (for example a bearer token).
    #[cfg_attr(
        target_family = "wasm",
        ::wasm_bindgen::prelude::wasm_bindgen(js_name = addHeader)
    )]
    pub fn add_header(&mut self, name: String, value: String) {
        self.headers.push((name, value));
    }
}

impl StompConnectOptions {
    /// Sets the STOMP virtual host with builder syntax.
    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    /// Sets the standard STOMP login with builder syntax.
    pub fn with_login(mut self, login: impl Into<String>) -> Self {
        self.login = Some(login.into());
        self
    }

    /// Sets the standard STOMP passcode with builder syntax.
    pub fn with_passcode(mut self, passcode: impl Into<String>) -> Self {
        self.passcode = Some(passcode.into());
        self
    }

    /// Adds an application-specific CONNECT header with builder syntax.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Sets the request/response reply timeout (`None` waits indefinitely). Enforced on native and
    /// wasm alike, bounding both command enqueueing and the reply wait.
    pub fn with_request_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.request_timeout = timeout;
        self
    }
}

/// A persistent STOMP client over one WebSocket connection. Cheap to clone (an `Arc`-backed handle
/// onto the actor); every clone shares the same connection. [`disconnect`](Self::disconnect) closes
/// it for every clone, and last-handle drop remains a best-effort graceful fallback.
#[derive(Clone)]
pub struct StompClientTransport {
    inner: Arc<TransportInner>,
}

impl StompClientTransport {
    /// Connects to a STOMP-over-WebSocket endpoint, performs the handshake, and starts the actor.
    pub async fn connect(url: impl AsRef<str>) -> Result<Self, ClientError<StompStatus>> {
        Self::connect_with_options(url, StompConnectOptions::default()).await
    }

    /// Connects with credentials and/or custom STOMP CONNECT headers.
    pub async fn connect_with_options(
        url: impl AsRef<str>,
        options: StompConnectOptions,
    ) -> Result<Self, ClientError<StompStatus>> {
        let socket = tokio_tungstenite_wasm::connect(url.as_ref())
            .await
            .map_err(net_err)?;
        let (mut write, mut read) = socket.split();

        let request_timeout = options.request_timeout;

        // Offer 1.2/1.1 and await CONNECTED before anything else may flow.
        let connect = connect_frame(options);

        write.send(to_message(connect)).await.map_err(net_err)?;

        await_connected(&mut read).await?;

        let (tx, rx) = mpsc::channel::<Command>(CHANNEL_DEPTH);
        let requests: Requests = Arc::new(Mutex::new(HashMap::new()));

        overseerd_axum::client::ws_rt::spawn(actor(write, read, rx, requests.clone()));

        Ok(Self {
            inner: Arc::new(TransportInner {
                tx,
                requests,
                next_id: AtomicU64::new(1),
                request_timeout,
            }),
        })
    }

    /// The command channel to the actor.
    fn tx(&self) -> &mpsc::Sender<Command> {
        &self.inner.tx
    }

    /// Whether the connection is still live. The actor holds the command-channel receiver for the
    /// life of the socket and drops it when the loop exits (a close, a fatal `ERROR` frame, or the
    /// last handle disconnecting), so a closed channel is exactly a dead connection.
    pub fn is_connected(&self) -> bool {
        !self.inner.tx.is_closed()
    }

    /// Gracefully disconnects this shared transport.
    ///
    /// The operation is idempotent and closes the actor for every clone, including transports held
    /// by already-created generated clients and subscriptions.
    pub async fn disconnect(&self) -> Result<(), ClientError<StompStatus>> {
        if !self.is_connected() {
            return Ok(());
        }

        let frame: Vec<u8> = DisconnectFrameBuilder::new("disconnect".to_owned())
            .build()
            .into();
        let (ack, ack_rx) = oneshot::channel();

        if self
            .tx()
            .send(Command::Disconnect {
                frame,
                ack: Some(ack),
            })
            .await
            .is_err()
        {
            return Ok(());
        }

        ack_rx.await.unwrap_or(Ok(()))
    }

    /// Starts a best-effort disconnect without waiting for the close frame to flush.
    ///
    /// Used by synchronous drop paths such as the browser [`Connection`](crate::client::Connection).
    pub fn disconnect_now(&self) {
        if !self.is_connected() {
            return;
        }

        let frame: Vec<u8> = DisconnectFrameBuilder::new("disconnect".to_owned())
            .build()
            .into();
        let _ = self.tx().try_send(Command::Disconnect { frame, ack: None });
    }

    /// A fresh, monotonically-increasing id (for subscriptions).
    fn next(&self, prefix: &str) -> String {
        format!(
            "{prefix}-{}",
            self.inner.next_id.fetch_add(1, Ordering::Relaxed)
        )
    }
}

impl MessageSend<Stomp> for StompClientTransport {
    async fn send(
        &self,
        destination: &str,
        body: StompBody,
    ) -> Result<(), ClientError<StompStatus>> {
        // The body is already codec-encoded; ship its bytes and content type verbatim.
        let mut builder = SendFrameBuilder::new(destination.to_owned());

        if let Some(content_type) = &body.content_type {
            builder = builder.content_type(content_type.clone());
        }

        let frame: Vec<u8> = builder.body(body.bytes.to_vec()).build().into();

        let (ack, ack_rx) = oneshot::channel();

        self.tx()
            .send(Command::Send { frame, ack })
            .await
            .map_err(|_| ClientError::ConnectionClosed)?;

        ack_rx.await.map_err(|_| ClientError::ConnectionClosed)?
    }
}

impl MessageRequest<Stomp> for StompClientTransport {
    async fn request(
        &self,
        destination: &str,
        body: StompBody,
    ) -> Result<StompBody, ClientError<StompStatus>> {
        // A client-chosen reply destination plus a correlation id: the server echoes the id on the
        // reply MESSAGE, which the actor demuxes to this call's channel.
        let correlation_id = self.next("corr");
        let reply_to = format!("/reply/{correlation_id}");

        let mut builder = SendFrameBuilder::new(destination.to_owned());

        builder = builder.add_custom_header("reply-to".to_owned(), reply_to);
        builder = builder.add_custom_header("correlation-id".to_owned(), correlation_id.clone());

        if let Some(content_type) = &body.content_type {
            builder = builder.content_type(content_type.clone());
        }

        let frame: Vec<u8> = builder.body(body.bytes.to_vec()).build().into();

        let (reply, reply_rx) = oneshot::channel();

        // Register synchronously, before the frame is even queued, so an immediate reply can never
        // miss its channel and there is no cross-channel ordering to get wrong.
        lock_requests(&self.inner.requests).insert(correlation_id.clone(), reply);

        // The guard removes this slot from the shared table when the call ends, however it ends — a
        // reply, a timeout, an outer abort, or a failed enqueue onto a dead connection. Removal is
        // idempotent (a no-op if a reply or the close-drain already took the slot), so it is always
        // correct to run and there is no state to arm/disarm: the table can never leak an entry.
        let _guard = RequestGuard::new(self.inner.requests.clone(), correlation_id.clone());

        // The deadline covers BOTH enqueueing and the reply wait: a full command channel or an actor
        // blocked writing to the socket must not let a request outlive `request_timeout` before the
        // wait even begins.
        await_reply(
            async {
                self.tx()
                    .send(Command::Request {
                        frame,
                        correlation_id,
                    })
                    .await
                    .map_err(|_| ClientError::ConnectionClosed)?;

                reply_rx.await.map_err(|_| ClientError::ConnectionClosed)?
            },
            self.inner.request_timeout,
        )
        .await
    }
}

/// Drop-guard that removes a request's correlation slot from the shared table when the call ends,
/// however it ends. Removal under the lock is idempotent, so it needs no arm/disarm state: a slot a
/// reply or the connection-close drain already took is simply not there, and one abandoned by a
/// timeout, an outer abort, or a failed enqueue is reclaimed here — the table can never leak.
struct RequestGuard {
    requests: Requests,
    correlation_id: String,
}

impl RequestGuard {
    fn new(requests: Requests, correlation_id: String) -> Self {
        Self {
            requests,
            correlation_id,
        }
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        // Direct, synchronous removal under the lock — it cannot be rejected or delayed by a full
        // command channel or a stalled actor, so the slot is always reclaimed promptly.
        let correlation_id = std::mem::take(&mut self.correlation_id);
        lock_requests(&self.requests).remove(&correlation_id);
    }
}

/// Runs the enqueue-and-await-reply future `fut`, bounded by `timeout` when set (`None` waits
/// indefinitely). Native uses [`tokio::time::timeout`]; a lapse yields [`ClientError::Timeout`].
#[cfg(not(target_family = "wasm"))]
async fn await_reply<F>(
    fut: F,
    timeout: Option<Duration>,
) -> Result<StompBody, ClientError<StompStatus>>
where
    F: Future<Output = Result<StompBody, ClientError<StompStatus>>>,
{
    let Some(timeout) = timeout else {
        return fut.await;
    };

    tokio::time::timeout(timeout, fut)
        .await
        .unwrap_or_else(|_elapsed| Err(ClientError::Timeout))
}

/// See the native variant. On wasm the deadline is enforced with a browser `setTimeout` timer
/// ([`wasm_timer::delay`]), so a missing reply resolves [`ClientError::Timeout`] in the browser too
/// rather than hanging forever.
#[cfg(target_family = "wasm")]
async fn await_reply<F>(
    fut: F,
    timeout: Option<Duration>,
) -> Result<StompBody, ClientError<StompStatus>>
where
    F: Future<Output = Result<StompBody, ClientError<StompStatus>>>,
{
    let Some(timeout) = timeout else {
        return fut.await;
    };

    let fut = std::pin::pin!(fut);
    let timer = std::pin::pin!(wasm_timer::delay(timeout));

    match futures::future::select(fut, timer).await {
        futures::future::Either::Left((received, _timer)) => received,

        futures::future::Either::Right(((), _fut)) => Err(ClientError::Timeout),
    }
}

/// A wasm-only reply deadline built on the browser `setTimeout`/`clearTimeout` — the STOMP transport
/// compiles for wasm but has no tokio timer there, so this gives `request` the same bounded behavior
/// as native. The pending timer is cleared once the delay resolves *or* is dropped (the reply won
/// the race), so a cancelled deadline never fires into a freed closure.
#[cfg(target_family = "wasm")]
mod wasm_timer {
    use std::time::Duration;

    use tokio::sync::oneshot;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_name = setTimeout)]
        fn set_timeout(handler: &Closure<dyn FnMut()>, timeout_ms: f64) -> f64;

        #[wasm_bindgen(js_name = clearTimeout)]
        fn clear_timeout(id: f64);
    }

    /// A scheduled `setTimeout` that clears itself on drop, so a fired or cancelled delay never
    /// leaves a live browser timer holding a freed closure.
    struct Scheduled {
        id: f64,
        _closure: Closure<dyn FnMut()>,
    }

    impl Drop for Scheduled {
        fn drop(&mut self) {
            clear_timeout(self.id);
        }
    }

    /// Signals the timer task to clear its `setTimeout` when the delay future is dropped before it
    /// fires (the reply won the race).
    struct CancelOnDrop(Option<oneshot::Sender<()>>);

    impl Drop for CancelOnDrop {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }

    /// The largest delay `setTimeout` honours: it stores the delay as a signed 32-bit millisecond
    /// value, so anything past `i32::MAX` ms (~24.8 days) overflows and fires almost immediately.
    const MAX_TIMEOUT_MS: u128 = i32::MAX as u128;

    /// Resolves after `duration` via the browser `setTimeout`.
    ///
    /// The `Closure` is `!Send`, yet `MessageRequest::request` requires a `Send` future even on wasm,
    /// so the closure and the pending timer live inside a spawned single-threaded task; the future
    /// this returns holds only `Send` channel endpoints. Dropping it signals that task to clear the
    /// timer.
    ///
    /// The delay is clamped to [`MAX_TIMEOUT_MS`]: a longer configured timeout would otherwise
    /// overflow `setTimeout`'s 32-bit millisecond field and fire near-instantly, so it is capped at
    /// the largest value the browser honours rather than misbehaving.
    pub async fn delay(duration: Duration) {
        let millis = duration.as_millis().min(MAX_TIMEOUT_MS) as f64;

        let (fired_tx, fired_rx) = oneshot::channel::<()>();
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

        wasm_bindgen_futures::spawn_local(async move {
            let (js_tx, js_rx) = oneshot::channel::<()>();
            let mut js_tx = Some(js_tx);

            let closure = Closure::wrap(Box::new(move || {
                if let Some(js_tx) = js_tx.take() {
                    let _ = js_tx.send(());
                }
            }) as Box<dyn FnMut()>);

            let id = set_timeout(&closure, millis);
            let _scheduled = Scheduled {
                id,
                _closure: closure,
            };

            // Fire when the timer elapses, or bail when the caller abandons the delay; either branch
            // drops `_scheduled` here, clearing the browser timer.
            tokio::select! {
                _ = js_rx => {
                    let _ = fired_tx.send(());
                }

                _ = cancel_rx => {}
            }
        });

        let _cancel_on_drop = CancelOnDrop(Some(cancel_tx));

        let _ = fired_rx.await;
    }
}

impl TopicSubscribe<Stomp> for StompClientTransport {
    async fn subscribe<M>(
        &self,
        destination: &str,
        decode: fn(StompBody) -> Result<M, CodecError>,
    ) -> Result<Subscription<Stomp, Self, M>, ClientError<StompStatus>>
    where
        M: Send + 'static,
    {
        let id = SubscriptionId(self.next("sub"));
        let (items_tx, items_rx) = mpsc::channel::<StompBody>(CHANNEL_DEPTH);
        let frame: Vec<u8> = SubscribeFrameBuilder::new(destination.to_owned(), id.0.clone())
            .build()
            .into();

        let (ack, ack_rx) = oneshot::channel();

        self.tx()
            .send(Command::Subscribe {
                id: id.clone(),
                frame,
                items: items_tx,
                ack,
            })
            .await
            .map_err(|_| ClientError::ConnectionClosed)?;

        ack_rx.await.map_err(|_| ClientError::ConnectionClosed)??;

        Ok(Subscription::new(id, items_rx, decode, self.clone()))
    }

    fn unsubscribe(&self, id: SubscriptionId) {
        let frame: Vec<u8> = UnsubscribeFrameBuilder::new(id.0.clone()).build().into();

        // Best-effort: dropping a subscription must not block, and a closed connection is fine.
        let _ = self.tx().try_send(Command::Unsubscribe { id, frame });
    }
}

/// The connection actor: writes queued commands and demuxes inbound frames until the socket closes
/// or an `ERROR` frame arrives.
async fn actor(
    mut write: WsWrite,
    mut read: WsRead,
    mut rx: mpsc::Receiver<Command>,
    requests: Requests,
) {
    let mut subs: HashMap<SubscriptionId, mpsc::Sender<StompBody>> = HashMap::new();
    let mut receipts: HashMap<String, Ack> = HashMap::new();

    loop {
        tokio::select! {
            command = rx.recv() => {
                let Some(command) = command else {
                    break;
                };

                match command {
                    Command::Send { frame, ack } => {
                        let result = write.send(to_message(frame)).await.map_err(net_err);
                        let _ = ack.send(result);
                    }

                    Command::Request { frame, correlation_id } => {
                        // The reply slot is already registered by the caller. On a write failure,
                        // fail it now (if the caller has not since abandoned it).
                        if let Err(error) = write.send(to_message(frame)).await.map_err(net_err)
                            && let Some(reply) = lock_requests(&requests).remove(&correlation_id)
                        {
                            let _ = reply.send(Err(error));
                        }
                    }

                    Command::Subscribe { id, frame, items, ack } => {
                        // Register before writing so an immediate MESSAGE can never miss its stream.
                        subs.insert(id, items);

                        let result = write.send(to_message(frame)).await.map_err(net_err);
                        let _ = ack.send(result);
                    }

                    Command::Unsubscribe { id, frame } => {
                        subs.remove(&id);

                        let _ = write.send(to_message(frame)).await;
                    }

                    Command::Disconnect { frame, ack } => {
                        // Explicit disconnect and last-handle drop share this path. Closing the
                        // receiver first makes every cloned sender immediately observe dead state.
                        rx.close();
                        let result = match write.send(to_message(frame)).await {
                            Ok(()) => write.close().await.map_err(net_err),
                            Err(error) => Err(net_err(error)),
                        };

                        if let Some(ack) = ack {
                            let _ = ack.send(result);
                        }

                        break;
                    }
                }
            }

            message = read.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if is_heartbeat(text.as_bytes()) {
                            continue;
                        }

                        if route(text.as_bytes().to_vec(), &subs, &mut receipts, &requests).is_break() {
                            break;
                        }
                    }

                    Some(Ok(Message::Binary(bytes))) => {
                        if route(bytes.to_vec(), &subs, &mut receipts, &requests).is_break() {
                            break;
                        }
                    }

                    Some(Ok(_)) => {}

                    Some(Err(_)) | None => break,
                }
            }
        }
    }

    // Fail everything outstanding: dropping the sub senders ends their streams; receipts and pending
    // requests resolve with a connection error.
    fail_receipts(receipts);
    fail_requests(&requests);
    drop(subs);
}

/// Routes one parsed server frame. Returns `Break` when the connection must close (an `ERROR`).
fn route(
    bytes: Vec<u8>,
    subs: &HashMap<SubscriptionId, mpsc::Sender<StompBody>>,
    receipts: &mut HashMap<String, Ack>,
    requests: &Requests,
) -> std::ops::ControlFlow<()> {
    use std::ops::ControlFlow::{Break, Continue};

    let frame = match ServerFrame::try_from(bytes) {
        Ok(frame) => frame,

        Err(_) => return Continue(()),
    };

    match frame {
        ServerFrame::Message(message) => {
            let body = StompBody {
                content_type: message.content_type().map(|c| c.value().to_owned()),
                bytes: message
                    .body()
                    .map(bytes::Bytes::copy_from_slice)
                    .unwrap_or_default(),
            };

            // A request/response reply is stamped with the reply sentinel subscription and a
            // `correlation-id`: resolve the awaiting call (terminal) and stop, so it never spills
            // into a subscription stream. The sentinel gate means a broadcast MESSAGE can never be
            // mistaken for a reply, even one carrying a colliding `correlation-id` header.
            if message.subscription().value() == REPLY_SUBSCRIPTION_ID
                && let Some(correlation_id) = message_correlation_id(&message)
                && let Some(reply) = lock_requests(requests).remove(&correlation_id)
            {
                let outcome = if message_is_error(&message) {
                    Err(ClientError::Remote(ErrorBody::new(
                        StompStatus::Handler,
                        body.bytes.to_vec(),
                    )))
                } else {
                    Ok(body)
                };

                let _ = reply.send(outcome);

                return Continue(());
            }

            let id = SubscriptionId(message.subscription().value().to_owned());

            if let Some(sender) = subs.get(&id) {
                let _ = sender.try_send(body);
            }

            Continue(())
        }

        ServerFrame::Receipt(receipt) => {
            let id = receipt.receipt_id().value().to_owned();

            if let Some(ack) = receipts.remove(&id) {
                let _ = ack.send(Ok(()));
            }

            Continue(())
        }

        ServerFrame::Error(error) => {
            let body = error.body().unwrap_or_default().to_vec();

            fail_receipts_with(receipts, || {
                ClientError::Remote(ErrorBody::new(StompStatus::Error, body.clone()))
            });
            fail_requests_with(requests, || {
                ClientError::Remote(ErrorBody::new(StompStatus::Error, body.clone()))
            });

            Break(())
        }

        ServerFrame::Connected(_) => Continue(()),
    }
}

/// The `correlation-id` custom header of a server `MESSAGE`, if present — the routing key for a
/// request/response reply.
fn message_correlation_id(message: &stomp_parser::server::MessageFrame<'_>) -> Option<String> {
    message
        .custom
        .iter()
        .find(|header| header.header_name() == "correlation-id")
        .map(|header| (*header.value()).to_owned())
}

/// Whether a reply `MESSAGE` is an *error* reply (the server marks a failed request handler): its
/// body becomes a [`ClientError::Remote`] resolving the awaiting call `Err`, not `Ok`.
fn message_is_error(message: &stomp_parser::server::MessageFrame<'_>) -> bool {
    message
        .custom
        .iter()
        .any(|header| header.header_name() == MESSAGE_ERROR_HEADER)
}

/// Reads frames until `CONNECTED` (ok), an `ERROR` (protocol error), or the socket closes.
async fn await_connected(read: &mut WsRead) -> Result<(), ClientError<StompStatus>> {
    loop {
        match read.next().await {
            Some(Ok(Message::Text(text))) => {
                if is_heartbeat(text.as_bytes()) {
                    continue;
                }

                return classify_handshake(text.as_bytes().to_vec());
            }

            Some(Ok(Message::Binary(bytes))) => return classify_handshake(bytes.to_vec()),

            Some(Ok(_)) => continue,

            Some(Err(error)) => return Err(net_err(error)),

            None => return Err(ClientError::ConnectionClosed),
        }
    }
}

/// Classifies the first server frame of the handshake: `CONNECTED` succeeds, anything else fails.
fn classify_handshake(bytes: Vec<u8>) -> Result<(), ClientError<StompStatus>> {
    match ServerFrame::try_from(bytes) {
        Ok(ServerFrame::Connected(_)) => Ok(()),

        Ok(ServerFrame::Error(error)) => Err(ClientError::Remote(ErrorBody::new(
            StompStatus::Error,
            error.body().unwrap_or_default().to_vec(),
        ))),

        _ => Err(ClientError::Remote(ErrorBody::new(
            StompStatus::Protocol,
            b"expected CONNECTED".to_vec(),
        ))),
    }
}

/// Fails every pending receipt with a plain connection-closed error.
fn fail_receipts(receipts: HashMap<String, Ack>) {
    for (_, ack) in receipts {
        let _ = ack.send(Err(ClientError::ConnectionClosed));
    }
}

/// Fails every pending receipt with a freshly-built error (used for a broker `ERROR`).
fn fail_receipts_with(
    receipts: &mut HashMap<String, Ack>,
    error: impl Fn() -> ClientError<StompStatus>,
) {
    for (_, ack) in receipts.drain() {
        let _ = ack.send(Err(error()));
    }
}

/// Fails every pending request with a plain connection-closed error, draining the shared table.
fn fail_requests(requests: &Requests) {
    for (_, reply) in lock_requests(requests).drain() {
        let _ = reply.send(Err(ClientError::ConnectionClosed));
    }
}

/// Fails every pending request with a freshly-built error (used for a broker `ERROR`), draining the
/// shared table.
fn fail_requests_with(requests: &Requests, error: impl Fn() -> ClientError<StompStatus>) {
    for (_, reply) in lock_requests(requests).drain() {
        let _ = reply.send(Err(error()));
    }
}

/// A bare newline (`\n` / `\r\n`) is a server heart-beat, not a frame.
fn is_heartbeat(bytes: &[u8]) -> bool {
    bytes == b"\n" || bytes == b"\r\n"
}

/// Wraps frame bytes in a WebSocket message: text when valid UTF-8, binary otherwise.
fn to_message(bytes: Vec<u8>) -> Message {
    match String::from_utf8(bytes) {
        Ok(text) => Message::text(text),

        Err(error) => Message::binary(error.into_bytes()),
    }
}

/// Maps a tungstenite error into a transport error.
fn net_err(error: WsError) -> ClientError<StompStatus> {
    ClientError::Transport(TransportError::Io(std::io::Error::other(error.to_string())))
}

/// Builds the CONNECT frame from the public options.
fn connect_frame(options: StompConnectOptions) -> Vec<u8> {
    let mut builder = ConnectFrameBuilder::new(
        options.host.unwrap_or_else(|| "localhost".to_owned()),
        StompVersions(vec![StompVersion::V1_2, StompVersion::V1_1]),
    );

    if let Some(login) = options.login {
        builder = builder.login(login);
    }

    if let Some(passcode) = options.passcode {
        builder = builder.passcode(passcode);
    }

    for (name, value) in options.headers {
        builder = builder.add_custom_header(name, value);
    }

    builder.build().into()
}

#[cfg(test)]
#[path = "transport/tests.rs"]
mod tests;
