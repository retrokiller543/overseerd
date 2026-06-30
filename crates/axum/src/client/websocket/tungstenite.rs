use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::{SinkExt, StreamExt};
use overseerd_client::{ClientError, ErrorBody};
use overseerd_transport::Error as TransportError;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Duration, MissedTickBehavior};
use tokio_tungstenite::tungstenite::Message;

use super::{
    WebsocketClient, WebsocketClientProtocol, WebsocketDecodes, WebsocketEncodes, WsStatus,
};

type Pending = oneshot::Sender<Result<String, ClientError<WsStatus>>>;

struct Command<K> {
    key: K,
    frame: String,
    reply: Pending,
}

/// A persistent tokio-tungstenite request/reply client for a websocket protocol `P`.
pub struct TokioTungsteniteWs<P: WebsocketClientProtocol> {
    tx: mpsc::Sender<Command<P::Key>>,
    next_key: Arc<AtomicU64>,
    _protocol: PhantomData<fn() -> P>,
}

impl<P> Clone for TokioTungsteniteWs<P>
where
    P: WebsocketClientProtocol,
{
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            next_key: Arc::clone(&self.next_key),
            _protocol: PhantomData,
        }
    }
}

impl<P> TokioTungsteniteWs<P>
where
    P: WebsocketClientProtocol<Frame = String>,
{
    /// Connects to a websocket endpoint and starts the background read/write actor.
    pub async fn connect(url: impl AsRef<str>) -> Result<Self, ClientError<WsStatus>> {
        let (socket, _) = tokio_tungstenite::connect_async(url.as_ref())
            .await
            .map_err(net_err)?;
        let (mut write, mut read) = socket.split();
        let (tx, mut rx) = mpsc::channel::<Command<P::Key>>(64);

        tokio::spawn(async move {
            let mut pending: HashMap<P::Key, Pending> = HashMap::new();
            let mut prune = tokio::time::interval(Duration::from_secs(30));

            prune.set_missed_tick_behavior(MissedTickBehavior::Delay);

            loop {
                tokio::select! {
                    command = rx.recv() => {
                        let Some(command) = command else {
                            break;
                        };

                        prune_pending(&mut pending);

                        if write.send(Message::Text(command.frame.into())).await.is_err() {
                            let _ = command.reply.send(Err(ClientError::ConnectionClosed));
                            fail_pending(pending, ClientError::ConnectionClosed);
                            break;
                        }

                        pending.insert(command.key, command.reply);
                    }

                    message = read.next() => {
                        match message {
                            Some(Ok(Message::Text(text))) => {
                                let text = text.to_string();

                                match P::reply_key(&text) {
                                    Ok(Some(key)) => {
                                        if let Some(reply) = pending.remove(&key) {
                                            let _ = reply.send(Ok(text));
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

                            Some(Ok(Message::Close(_))) | None => {
                                fail_pending(pending, ClientError::ConnectionClosed);
                                break;
                            }

                            Some(Ok(_)) => {}

                            Some(Err(error)) => {
                                fail_pending(pending, net_err(error));
                                break;
                            }
                        }
                    }

                    _ = prune.tick() => {
                        prune_pending(&mut pending);
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
}

impl<P, Req, Resp> WebsocketClient<P, Req, Resp> for TokioTungsteniteWs<P>
where
    P: WebsocketClientProtocol<Frame = String>,
    P: WebsocketEncodes<Req> + WebsocketDecodes<Resp>,
    Req: Send,
    Resp: Send,
{
    async fn websocket_call(
        &self,
        destination: &'static str,
        payload: Req,
    ) -> Result<Resp, ClientError<WsStatus>>
    where
        P: WebsocketEncodes<Req> + WebsocketDecodes<Resp>,
        Req: Send,
        Resp: Send,
    {
        let key = P::next_key(self.next_key.fetch_add(1, Ordering::Relaxed));
        let frame = P::encode_call(destination, &key, payload)
            .map_err(|e| ClientError::Encode(e.to_string()))?;
        let (reply, recv) = oneshot::channel();

        self.tx
            .send(Command { key, frame, reply })
            .await
            .map_err(|_| ClientError::ConnectionClosed)?;

        let frame = recv.await.map_err(|_| ClientError::ConnectionClosed)??;

        P::decode_reply(frame)
    }
}

fn fail_pending<K>(pending: HashMap<K, Pending>, error: ClientError<WsStatus>) {
    for (_, reply) in pending {
        let _ = reply.send(Err(clone_client_error(&error)));
    }
}

fn prune_pending<K>(pending: &mut HashMap<K, Pending>) {
    pending.retain(|_, reply| !reply.is_closed());
}

fn clone_client_error(error: &ClientError<WsStatus>) -> ClientError<WsStatus> {
    match error {
        ClientError::Transport(_) => ClientError::Transport(TransportError::Closed),
        ClientError::Encode(message) => ClientError::Encode(message.clone()),
        ClientError::Decode(message) => ClientError::Decode(message.clone()),
        ClientError::Remote(body) => {
            ClientError::Remote(ErrorBody::new(body.code(), body.raw().to_vec()))
        }
        ClientError::ConnectionClosed => ClientError::ConnectionClosed,
    }
}

fn net_err<T>(error: T) -> ClientError<WsStatus>
where
    T: std::fmt::Display,
{
    ClientError::Transport(TransportError::Io(std::io::Error::other(error.to_string())))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tokio::sync::oneshot;

    use super::{Pending, prune_pending};

    #[test]
    fn prune_pending_removes_calls_after_receiver_is_dropped() {
        let (closed_tx, closed_rx) = oneshot::channel();
        let (open_tx, _open_rx) = oneshot::channel();
        let mut pending: HashMap<u64, Pending> = HashMap::new();

        pending.insert(1, closed_tx);
        pending.insert(2, open_tx);
        drop(closed_rx);

        prune_pending(&mut pending);

        assert!(!pending.contains_key(&1));
        assert!(pending.contains_key(&2));
    }
}
