//! The tokio-tungstenite STOMP transport actor.
//!
//! [`StompClientTransport::connect`] performs the CONNECT/CONNECTED handshake, then spawns a
//! background task that owns the socket and demuxes inbound frames into three routing tables:
//! subscription id → durable `MESSAGE` stream, receipt id → terminal `RECEIPT`, and a fatal
//! `ERROR`/close that fails everything outstanding (the direct analogue of the RPC client's read
//! loop clearing its call table on disconnect).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::{SinkExt, StreamExt};
use overseerd_client::{ClientError, ErrorBody};
use overseerd_transport::{CodecError, Error as TransportError};
use stomp_parser::client::{ConnectFrameBuilder, SendFrameBuilder, SubscribeFrameBuilder, UnsubscribeFrameBuilder};
use stomp_parser::headers::{StompVersion, StompVersions};
use stomp_parser::server::ServerFrame;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use super::{StompBody, StompSend, StompStatus, StompSubscribe, Subscription, SubscriptionId};

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
}

/// A persistent STOMP client over one WebSocket connection. Cheap to clone (an `Arc`-backed handle
/// onto the actor); every clone shares the same connection.
#[derive(Clone)]
pub struct StompClientTransport {
    tx: mpsc::Sender<Command>,
    next_id: Arc<AtomicU64>,
}

impl StompClientTransport {
    /// Connects to a STOMP-over-WebSocket endpoint, performs the handshake, and starts the actor.
    pub async fn connect(url: impl AsRef<str>) -> Result<Self, ClientError<StompStatus>> {
        let (socket, _) = tokio_tungstenite::connect_async(url.as_ref())
            .await
            .map_err(net_err)?;
        let (mut write, mut read) = socket.split();

        // Offer 1.2/1.1 and await CONNECTED before anything else may flow.
        let connect: Vec<u8> = ConnectFrameBuilder::new(
            "localhost".to_owned(),
            StompVersions(vec![StompVersion::V1_2, StompVersion::V1_1]),
        )
        .build()
        .into();

        write.send(to_message(connect)).await.map_err(net_err)?;

        await_connected(&mut read).await?;

        let (tx, rx) = mpsc::channel::<Command>(CHANNEL_DEPTH);

        tokio::spawn(actor(write, read, rx));

        Ok(Self {
            tx,
            next_id: Arc::new(AtomicU64::new(1)),
        })
    }

    /// A fresh, monotonically-increasing id (for subscriptions).
    fn next(&self, prefix: &str) -> String {
        format!("{prefix}-{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }
}

impl<Req> StompSend<Req> for StompClientTransport
where
    Req: serde::Serialize + Send,
{
    async fn stomp_send(
        &self,
        destination: &'static str,
        payload: Req,
    ) -> Result<(), ClientError<StompStatus>> {
        let body = serde_json::to_vec(&payload).map_err(|e| ClientError::Encode(e.to_string()))?;
        let frame: Vec<u8> = SendFrameBuilder::new(destination.to_owned())
            .content_type("application/json".to_owned())
            .body(body)
            .build()
            .into();

        let (ack, ack_rx) = oneshot::channel();

        self.tx
            .send(Command::Send { frame, ack })
            .await
            .map_err(|_| ClientError::ConnectionClosed)?;

        ack_rx.await.map_err(|_| ClientError::ConnectionClosed)?
    }
}

impl StompSubscribe for StompClientTransport {
    async fn stomp_subscribe<M>(
        &self,
        destination: &'static str,
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

        self.tx
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
        let _ = self.tx.try_send(Command::Unsubscribe { id, frame });
    }
}

/// The connection actor: writes queued commands and demuxes inbound frames until the socket closes
/// or an `ERROR` frame arrives.
async fn actor(
    mut write: futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    mut read: impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
    mut rx: mpsc::Receiver<Command>,
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
                bytes: message.body().map(bytes::Bytes::copy_from_slice).unwrap_or_default(),
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
async fn await_connected(
    read: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
) -> Result<(), ClientError<StompStatus>> {
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
fn net_err(error: tokio_tungstenite::tungstenite::Error) -> ClientError<StompStatus> {
    ClientError::Transport(TransportError::Io(std::io::Error::other(error.to_string())))
}
