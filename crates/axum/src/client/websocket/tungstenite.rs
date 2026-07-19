use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::{SinkExt, StreamExt};
use overseerd_client::{ClientError, ErrorBody};
use overseerd_transport::Error as TransportError;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite_wasm::Message;

use super::{
    WebsocketClient, WebsocketClientProtocol, WebsocketDecodes, WebsocketEncodes, WsClientFrame,
};

type Pending<S> = oneshot::Sender<Result<WsClientFrame, ClientError<S>>>;
type SendAck<S> = oneshot::Sender<Result<(), ClientError<S>>>;

enum Command<K, S> {
    Request {
        key: K,
        frame: WsClientFrame,
        reply: Pending<S>,
    },
    Send {
        frame: WsClientFrame,
        ack: SendAck<S>,
    },
}

/// Options for a correlated WebSocket client connection.
#[derive(Clone, Debug, Default)]
pub struct WebsocketConnectOptions {
    subprotocols: Vec<String>,
}

impl WebsocketConnectOptions {
    /// Offers RFC 6455 subprotocols in client preference order.
    pub fn with_subprotocols<I, S>(mut self, subprotocols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.subprotocols = subprotocols.into_iter().map(Into::into).collect();

        self
    }
}

/// A persistent cross-target request/reply client for WebSocket protocol `P`.
pub struct TokioTungsteniteWs<P: WebsocketClientProtocol> {
    tx: mpsc::Sender<Command<P::Key, P::Status>>,
    next_key: Arc<AtomicU64>,
    _protocol: PhantomData<fn() -> P>,
}

impl<P: WebsocketClientProtocol> Clone for TokioTungsteniteWs<P> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            next_key: Arc::clone(&self.next_key),
            _protocol: PhantomData,
        }
    }
}

impl<P: WebsocketClientProtocol> TokioTungsteniteWs<P> {
    /// Connects without offering an RFC 6455 subprotocol.
    pub async fn connect(url: impl AsRef<str>) -> Result<Self, ClientError<P::Status>> {
        Self::connect_with_options(url, WebsocketConnectOptions::default()).await
    }

    /// Connects with explicit WebSocket options and starts the background correlation actor.
    pub async fn connect_with_options(
        url: impl AsRef<str>,
        options: WebsocketConnectOptions,
    ) -> Result<Self, ClientError<P::Status>> {
        let protocols: Vec<&str> = options.subprotocols.iter().map(String::as_str).collect();
        let socket = if protocols.is_empty() {
            tokio_tungstenite_wasm::connect(url.as_ref()).await
        } else {
            tokio_tungstenite_wasm::connect_with_protocols(url.as_ref(), &protocols).await
        }
        .map_err(net_err)?;
        let (mut write, mut read) = socket.split();
        let (tx, mut rx) = mpsc::channel::<Command<P::Key, P::Status>>(64);

        crate::client::ws_rt::spawn(async move {
            let mut pending: HashMap<P::Key, Pending<P::Status>> = HashMap::new();
            loop {
                tokio::select! {
                    command = rx.recv() => {
                        let Some(command) = command else {
                            break;
                        };

                        prune_pending(&mut pending);

                        match command {
                            Command::Request { key, frame, reply } => {
                                if write.send(into_message(frame)).await.is_err() {
                                    let _ = reply.send(Err(ClientError::ConnectionClosed));
                                    fail_pending(pending, ClientError::ConnectionClosed);

                                    break;
                                }

                                pending.insert(key, reply);
                            }

                            Command::Send { frame, ack } => {
                                if write.send(into_message(frame)).await.is_err() {
                                    let _ = ack.send(Err(ClientError::ConnectionClosed));
                                    fail_pending(pending, ClientError::ConnectionClosed);

                                    break;
                                }

                                let _ = ack.send(Ok(()));
                            }
                        }
                    }

                    message = read.next() => {
                        match message {
                            Some(Ok(Message::Text(text))) => {
                                correlate::<P>(
                                    WsClientFrame::Text(text.to_string()),
                                    &mut pending,
                                );
                            }

                            Some(Ok(Message::Binary(bytes))) => {
                                correlate::<P>(WsClientFrame::Binary(bytes.to_vec()), &mut pending);
                            }

                            Some(Ok(Message::Close(_))) | None => {
                                fail_pending(pending, ClientError::ConnectionClosed);

                                break;
                            }

                            Some(Err(error)) => {
                                fail_pending(pending, net_err(error));

                                break;
                            }
                        }
                    }

                }
            }
        });

        Ok(Self {
            tx,
            next_key: Arc::new(AtomicU64::new(1)),
            _protocol: PhantomData,
        })
    }

    /// Sends one uncorrelated message and resolves when the frame is written to the socket actor.
    pub async fn send_message<Req>(
        &self,
        destination: &str,
        payload: Req,
    ) -> Result<(), ClientError<P::Status>>
    where
        P: WebsocketEncodes<Req>,
        Req: Send,
    {
        let frame = P::encode_send(destination, payload)
            .map_err(|error| ClientError::Encode(error.to_string()))?;
        let (ack, recv) = oneshot::channel();

        self.tx
            .send(Command::Send { frame, ack })
            .await
            .map_err(|_| ClientError::ConnectionClosed)?;

        recv.await.map_err(|_| ClientError::ConnectionClosed)?
    }
}

impl<P, Req, Resp> WebsocketClient<P, Req, Resp> for TokioTungsteniteWs<P>
where
    P: WebsocketClientProtocol + WebsocketEncodes<Req> + WebsocketDecodes<Resp>,
    Req: Send,
    Resp: Send,
{
    async fn websocket_call(
        &self,
        destination: &str,
        payload: Req,
    ) -> Result<Resp, ClientError<P::Status>>
    where
        Req: Send,
        Resp: Send,
    {
        let key = P::next_key(self.next_key.fetch_add(1, Ordering::Relaxed));
        let frame = P::encode_call(destination, &key, payload)
            .map_err(|error| ClientError::Encode(error.to_string()))?;
        let (reply, recv) = oneshot::channel();

        self.tx
            .send(Command::Request { key, frame, reply })
            .await
            .map_err(|_| ClientError::ConnectionClosed)?;

        let frame = recv.await.map_err(|_| ClientError::ConnectionClosed)??;

        P::decode_reply(frame)
    }
}

fn correlate<P: WebsocketClientProtocol>(
    frame: WsClientFrame,
    pending: &mut HashMap<P::Key, Pending<P::Status>>,
) {
    match P::reply_key(&frame) {
        Ok(Some(key)) => {
            if let Some(reply) = pending.remove(&key) {
                let _ = reply.send(Ok(frame));
            }
        }

        Ok(None) => {}

        Err(error) => {
            tracing::warn!(
                target: "overseerd::axum",
                %error,
                "ws response frame could not be correlated"
            );
        }
    }
}

fn into_message(frame: WsClientFrame) -> Message {
    match frame {
        WsClientFrame::Text(text) => Message::Text(text.into()),
        WsClientFrame::Binary(bytes) => Message::Binary(bytes.into()),
    }
}

fn fail_pending<K, S>(pending: HashMap<K, Pending<S>>, error: ClientError<S>)
where
    S: Copy,
{
    for (_, reply) in pending {
        let _ = reply.send(Err(clone_client_error(&error)));
    }
}

fn prune_pending<K, S>(pending: &mut HashMap<K, Pending<S>>) {
    pending.retain(|_, reply| !reply.is_closed());
}

fn clone_client_error<S: Copy>(error: &ClientError<S>) -> ClientError<S> {
    match error {
        ClientError::Transport(_) => ClientError::Transport(TransportError::Closed),
        ClientError::Encode(message) => ClientError::Encode(message.clone()),
        ClientError::Decode(message) => ClientError::Decode(message.clone()),
        ClientError::Remote(body) => {
            ClientError::Remote(ErrorBody::new(body.code(), body.raw().to_vec()))
        }
        ClientError::ConnectionClosed => ClientError::ConnectionClosed,
        ClientError::Timeout => ClientError::Timeout,
    }
}

fn net_err<S, T>(error: T) -> ClientError<S>
where
    T: std::fmt::Display,
{
    ClientError::Transport(TransportError::Io(std::io::Error::other(error.to_string())))
}

#[cfg(test)]
mod tests;
