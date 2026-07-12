//! The tokio-tungstenite STOMP transport actor.
//!
//! [`StompClientTransport::connect`] performs the CONNECT/CONNECTED handshake, then spawns a
//! background task that owns the socket and demuxes inbound frames into three routing tables:
//! subscription id → durable `MESSAGE` stream, receipt id → terminal `RECEIPT`, and a fatal
//! `ERROR`/close that fails everything outstanding (the direct analogue of the RPC client's read
//! loop clearing its call table on disconnect).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use overseerd_client::{ClientError, ErrorBody};
use overseerd_transport::{CodecError, Error as TransportError};
use stomp_parser::client::{
    ConnectFrameBuilder, DisconnectFrameBuilder, SendFrameBuilder, SubscribeFrameBuilder,
    UnsubscribeFrameBuilder,
};
use stomp_parser::headers::{StompVersion, StompVersions};
use stomp_parser::server::ServerFrame;
use tokio::sync::{mpsc, oneshot};
// One unified WebSocket type across native and wasm — the socket naming (`MaybeTlsStream<TcpStream>`
// on native, the JS `WebSocket` on wasm) is hidden, so this transport is target-agnostic.
use tokio_tungstenite_wasm::{Error as WsError, Message, WebSocketStream};

use super::{StompBody, StompSend, StompStatus, StompSubscribe, Subscription, SubscriptionId};

/// The write and read halves of a connected WebSocket, split for the actor loop.
type WsWrite = SplitSink<WebSocketStream, Message>;
type WsRead = SplitStream<WebSocketStream>;

/// The outbound-frame and inbound-message channel depths.
const CHANNEL_DEPTH: usize = 64;

/// An acknowledgement that a queued command reached (or failed to reach) the socket.
type Ack = oneshot::Sender<Result<(), ClientError<StompStatus>>>;

/// A command from a client handle to the connection actor.
enum Command {
    Send {
        frame: Vec<u8>,
        ack: Ack,
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
    next_id: AtomicU64,
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

/// Authentication and custom headers sent on the STOMP `CONNECT` frame.
#[cfg_attr(target_family = "wasm", ::wasm_bindgen::prelude::wasm_bindgen)]
#[derive(Clone, Default)]
pub struct StompConnectOptions {
    host: Option<String>,
    login: Option<String>,
    passcode: Option<String>,
    headers: Vec<(String, String)>,
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

        // Offer 1.2/1.1 and await CONNECTED before anything else may flow.
        let connect = connect_frame(options);

        write.send(to_message(connect)).await.map_err(net_err)?;

        await_connected(&mut read).await?;

        let (tx, rx) = mpsc::channel::<Command>(CHANNEL_DEPTH);

        crate::client::ws_rt::spawn(actor(write, read, rx));

        Ok(Self {
            inner: Arc::new(TransportInner {
                tx,
                next_id: AtomicU64::new(1),
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

impl StompSend for StompClientTransport {
    async fn stomp_send(
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

impl StompSubscribe for StompClientTransport {
    async fn stomp_subscribe<M>(
        &self,
        destination: &str,
        decode: fn(StompBody) -> Result<M, CodecError>,
    ) -> Result<Subscription<Self, M>, ClientError<StompStatus>>
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
async fn actor(mut write: WsWrite, mut read: WsRead, mut rx: mpsc::Receiver<Command>) {
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

                        if route(text.as_bytes().to_vec(), &subs, &mut receipts).is_break() {
                            break;
                        }
                    }

                    Some(Ok(Message::Binary(bytes))) => {
                        if route(bytes.to_vec(), &subs, &mut receipts).is_break() {
                            break;
                        }
                    }

                    Some(Ok(_)) => {}

                    Some(Err(_)) | None => break,
                }
            }
        }
    }

    // Fail everything outstanding: dropping the sub senders ends their streams; receipts resolve
    // with a connection error.
    fail_receipts(receipts);
    drop(subs);
}

/// Routes one parsed server frame. Returns `Break` when the connection must close (an `ERROR`).
fn route(
    bytes: Vec<u8>,
    subs: &HashMap<SubscriptionId, mpsc::Sender<StompBody>>,
    receipts: &mut HashMap<String, Ack>,
) -> std::ops::ControlFlow<()> {
    use std::ops::ControlFlow::{Break, Continue};

    let frame = match ServerFrame::try_from(bytes) {
        Ok(frame) => frame,

        Err(_) => return Continue(()),
    };

    match frame {
        ServerFrame::Message(message) => {
            let id = SubscriptionId(message.subscription().value().to_owned());
            let body = StompBody {
                content_type: message.content_type().map(|c| c.value().to_owned()),
                bytes: message
                    .body()
                    .map(bytes::Bytes::copy_from_slice)
                    .unwrap_or_default(),
            };

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

            Break(())
        }

        ServerFrame::Connected(_) => Continue(()),
    }
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
mod tests;
