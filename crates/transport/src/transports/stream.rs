use std::{collections::HashMap, sync::Arc};

use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{Mutex, mpsc},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, instrument, trace, warn};

use crate::{
    error::{Error, Result},
    frame::{CallId, CallResult, IncomingCall, PeerInfo},
    protocol::{
        WireMessage, WireResponse,
        codec::{read_message, write_message},
    },
    status::StatusCode,
    transport::{Connection, Respond, RespondStream, ResponseSink},
};

/// Inbound items buffered per streaming call before backpressure kicks in.
const INBOUND_BUFFER: usize = 32;

/// Per-call routing state owned exclusively by the connection's read loop.
struct CallSlot {
    /// `Some` while the call accepts inbound items (client/bidi streaming).
    inbound: Option<mpsc::Sender<Vec<u8>>>,
    /// Fired on `StreamCancel` for this call or when the connection drops.
    cancel: CancellationToken,
}

/// A connection over any reliable, ordered byte stream (TCP, Unix sockets).
///
/// The write half is shared with each responder behind a mutex so a responder
/// (or streaming sink) can outlive `recv` and write frames concurrently with
/// the read loop; the lock is held only for one frame write. The call table is
/// owned solely by the read loop — completions are reported back over a channel
/// rather than through a shared lock — so demuxing inbound frames needs no
/// cross-task synchronization beyond that write mutex.
pub struct StreamConnection<R, W> {
    read: R,
    write: Arc<Mutex<W>>,
    peer: PeerInfo,
    calls: HashMap<CallId, CallSlot>,
    completions_tx: mpsc::UnboundedSender<CallId>,
    completions_rx: mpsc::UnboundedReceiver<CallId>,
}

/// Responds to one inbound call on a stream connection. Owns the call's wire
/// id and a shared handle to the connection's write half. Becomes a
/// [`StreamSink`] for streaming responses via [`RespondStream`].
pub struct StreamResponder<W> {
    write: Arc<Mutex<W>>,
    id: CallId,
    completions_tx: mpsc::UnboundedSender<CallId>,
}

/// The outbound sink for a streaming call, writing `StreamItem`/`StreamEnd`/
/// `StreamError` frames tagged with the call's id.
pub struct StreamSink<W> {
    write: Arc<Mutex<W>>,
    id: CallId,
    completions_tx: mpsc::UnboundedSender<CallId>,
}

impl<R, W> StreamConnection<R, W> {
    pub fn new(read: R, write: W, peer: PeerInfo) -> Self {
        let (completions_tx, completions_rx) = mpsc::unbounded_channel();

        Self {
            read,
            write: Arc::new(Mutex::new(write)),
            peer,
            calls: HashMap::new(),
            completions_tx,
            completions_rx,
        }
    }

    /// Cancels every in-flight call. Called when the peer disconnects or the
    /// connection is dropped, so handlers observing their token unwind.
    fn cancel_all(&self) {
        for slot in self.calls.values() {
            slot.cancel.cancel();
        }
    }
}

impl<R, W> Connection for StreamConnection<R, W>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    type Responder = StreamResponder<W>;

    fn peer(&self) -> &PeerInfo {
        &self.peer
    }

    #[instrument(level = "trace", skip_all, fields(id = tracing::field::Empty, path = tracing::field::Empty))]
    async fn recv(&mut self) -> Result<Option<(IncomingCall, StreamResponder<W>)>> {
        loop {
            tokio::select! {
                biased;

                // Reap finished calls first so the table stays bounded.
                Some(done) = self.completions_rx.recv() => {
                    self.calls.remove(&done);
                }

                msg = read_message(&mut self.read) => {
                    match msg {
                        Ok(WireMessage::Request(req)) => {
                            let id = req.id;

                            tracing::Span::current()
                                .record("id", id)
                                .record("path", tracing::field::display(&req.path));

                            debug!("call received");

                            let cancel = CancellationToken::new();
                            let requests = if req.streaming_input {
                                let (tx, rx) = mpsc::channel(INBOUND_BUFFER);

                                self.calls.insert(id, CallSlot { inbound: Some(tx), cancel: cancel.clone() });

                                Some(rx)
                            } else {
                                self.calls.insert(id, CallSlot { inbound: None, cancel: cancel.clone() });

                                None
                            };

                            let call = IncomingCall {
                                path: req.path,
                                payload: req.payload,
                                requests,
                                cancel,
                            };
                            let responder = StreamResponder {
                                write: Arc::clone(&self.write),
                                id,
                                completions_tx: self.completions_tx.clone(),
                            };

                            return Ok(Some((call, responder)));
                        }

                        Ok(WireMessage::StreamItem { id, payload }) => {
                            let sender = self.calls.get(&id).and_then(|s| s.inbound.clone());

                            if let Some(tx) = sender {
                                // Awaiting here is the inbound backpressure path; it also
                                // head-of-line-blocks other calls (accepted for v1).
                                let _ = tx.send(payload).await;
                            }
                        }

                        Ok(WireMessage::StreamEnd { id }) => {
                            if let Some(slot) = self.calls.get_mut(&id) {
                                slot.inbound = None;
                            }
                        }

                        Ok(WireMessage::StreamCancel { id }) => {
                            if let Some(slot) = self.calls.get(&id) {
                                slot.cancel.cancel();
                            }
                        }

                        Err(Error::Io(e)) if is_disconnect(&e) => {
                            debug!(error = %e, "peer disconnected");
                            self.cancel_all();

                            return Ok(None);
                        }

                        Ok(WireMessage::Response(_))
                        | Ok(WireMessage::StreamError { .. }) => {
                            warn!("unexpected server-bound message from peer");
                            self.cancel_all();

                            return Err(Error::UnexpectedMessage);
                        }

                        Err(e) => {
                            warn!(error = %e, "frame read error");
                            self.cancel_all();

                            return Err(e);
                        }
                    }
                }
            }
        }
    }
}

impl<R, W> Drop for StreamConnection<R, W> {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

impl<W> Respond for StreamResponder<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    #[instrument(level = "trace", skip_all, fields(id = self.id))]
    async fn respond(self, outcome: CallResult) -> Result<()> {
        trace!("writing response");

        let msg = WireMessage::Response(WireResponse::new(self.id, outcome));

        {
            let mut write = self.write.lock().await;

            write_message(&mut *write, &msg).await?;
        }

        let _ = self.completions_tx.send(self.id);

        trace!("response written");

        Ok(())
    }
}

impl<W> RespondStream for StreamResponder<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    type Sink = StreamSink<W>;

    fn into_sink(self) -> StreamSink<W> {
        StreamSink {
            write: self.write,
            id: self.id,
            completions_tx: self.completions_tx,
        }
    }
}

impl<W> StreamSink<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    /// Writes one frame under the shared write lock, held only for the write.
    async fn write_frame(&self, msg: &WireMessage) -> Result<()> {
        let mut write = self.write.lock().await;

        write_message(&mut *write, msg).await
    }
}

impl<W> ResponseSink for StreamSink<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    #[instrument(level = "trace", skip_all, fields(id = self.id))]
    async fn send(&mut self, item: Vec<u8>) -> Result<()> {
        trace!("writing stream item");

        self.write_frame(&WireMessage::StreamItem {
            id: self.id,
            payload: item,
        })
        .await
    }

    #[instrument(level = "trace", skip_all, fields(id = self.id))]
    async fn error(self, code: StatusCode, body: Vec<u8>) -> Result<()> {
        trace!("writing stream error");

        self.write_frame(&WireMessage::StreamError {
            id: self.id,
            code,
            body,
        })
        .await?;

        let _ = self.completions_tx.send(self.id);

        Ok(())
    }

    #[instrument(level = "trace", skip_all, fields(id = self.id))]
    async fn finish(self) -> Result<()> {
        trace!("writing stream end");

        self.write_frame(&WireMessage::StreamEnd { id: self.id })
            .await?;

        let _ = self.completions_tx.send(self.id);

        Ok(())
    }
}

/// Distinguishes an orderly peer disconnect from a genuine I/O failure.
fn is_disconnect(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
    )
}
