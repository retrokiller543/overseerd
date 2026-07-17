use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{Mutex, mpsc},
    task::JoinSet,
    time::timeout,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, instrument, trace, warn};

use crate::{
    error::{Error, Result},
    frame::{CallId, CallResult, IncomingCall, PeerInfo},
    protocol::{
        WireMessage, WireResponse,
        codec::{FrameConfig, MessageReader, write_message},
    },
    status::{PredefinedCode, StatusCode},
    transport::{Connection, Respond, RespondStream, ResponseSink},
};

/// Inbound items buffered per streaming call before backpressure kicks in.
const INBOUND_BUFFER: usize = 32;

/// Default upper bound on calls concurrently tracked by one stream connection.
pub const DEFAULT_MAX_IN_FLIGHT_CALLS: usize = 256;

/// Default deadline for writing a transport-generated control response.
pub const DEFAULT_CONTROL_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Resource limits for a reliable byte-stream connection.
#[derive(Clone, Copy, Debug)]
pub struct StreamConfig {
    frame: FrameConfig,
    max_in_flight_calls: usize,
    control_write_timeout: Duration,
}

impl StreamConfig {
    pub fn new(
        frame: FrameConfig,
        max_in_flight_calls: usize,
        control_write_timeout: Duration,
    ) -> Self {
        assert!(
            max_in_flight_calls > 0,
            "maximum in-flight calls must be non-zero"
        );
        assert!(
            !control_write_timeout.is_zero(),
            "control write timeout must be non-zero"
        );

        Self {
            frame,
            max_in_flight_calls,
            control_write_timeout,
        }
    }

    pub fn frame(self) -> FrameConfig {
        self.frame
    }

    pub fn max_in_flight_calls(self) -> usize {
        self.max_in_flight_calls
    }

    pub fn control_write_timeout(self) -> Duration {
        self.control_write_timeout
    }
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self::new(
            FrameConfig::default(),
            DEFAULT_MAX_IN_FLIGHT_CALLS,
            DEFAULT_CONTROL_WRITE_TIMEOUT,
        )
    }
}

/// Per-call routing state owned exclusively by the connection's read loop.
struct CallSlot {
    /// `Some` while the call accepts inbound items (client/bidi streaming).
    inbound: Option<mpsc::Sender<Vec<u8>>>,
    /// Fired on `StreamCancel` for this call or when the connection drops.
    cancel: CancellationToken,
    /// Ensures exactly one terminal frame wins between the handler and transport.
    active: Arc<AtomicBool>,
    /// Reserves the id while a transport-generated terminal frame is pending.
    tombstone: bool,
}

struct ControlCompletion {
    id: CallId,
    result: Result<()>,
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
    reader: MessageReader<R>,
    write: Arc<Mutex<W>>,
    peer: PeerInfo,
    config: StreamConfig,
    calls: HashMap<CallId, CallSlot>,
    completions_tx: mpsc::UnboundedSender<CallId>,
    completions_rx: mpsc::UnboundedReceiver<CallId>,
    control_tasks: JoinSet<ControlCompletion>,
}

/// Responds to one inbound call on a stream connection. Owns the call's wire
/// id and a shared handle to the connection's write half. Becomes a
/// [`StreamSink`] for streaming responses via [`RespondStream`].
pub struct StreamResponder<W> {
    write: Arc<Mutex<W>>,
    id: CallId,
    completions_tx: mpsc::UnboundedSender<CallId>,
    active: Arc<AtomicBool>,
}

/// The outbound sink for a streaming call, writing `StreamItem`/`StreamEnd`/
/// `StreamError` frames tagged with the call's id.
pub struct StreamSink<W> {
    write: Arc<Mutex<W>>,
    id: CallId,
    completions_tx: mpsc::UnboundedSender<CallId>,
    active: Arc<AtomicBool>,
}

impl<R, W> StreamConnection<R, W> {
    pub fn new(read: R, write: W, peer: PeerInfo) -> Self {
        Self::with_config(read, write, peer, StreamConfig::default())
    }

    pub fn with_config(read: R, write: W, peer: PeerInfo, config: StreamConfig) -> Self {
        let (completions_tx, completions_rx) = mpsc::unbounded_channel();

        Self {
            reader: MessageReader::with_config(read, config.frame),
            write: Arc::new(Mutex::new(write)),
            peer,
            config,
            calls: HashMap::new(),
            completions_tx,
            completions_rx,
            control_tasks: JoinSet::new(),
        }
    }

    pub fn config(&self) -> StreamConfig {
        self.config
    }

    /// Cancels every in-flight call. Called when the peer disconnects or the
    /// connection is dropped, so handlers observing their token unwind.
    fn cancel_all(&self) {
        for slot in self.calls.values() {
            slot.cancel.cancel();
            slot.active.store(false, Ordering::Release);
        }
    }
}

impl<R, W> StreamConnection<R, W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    fn reject_inbound_overflow(&mut self, id: CallId) -> Result<()> {
        let Some(slot) = self.calls.get_mut(&id) else {
            return Ok(());
        };

        slot.cancel.cancel();
        slot.inbound = None;

        if !slot.active.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        slot.tombstone = true;

        while let Some(result) = self.control_tasks.try_join_next() {
            self.finish_control_task(result)?;
        }

        if self.control_tasks.len() >= self.config.max_in_flight_calls {
            self.cancel_all();

            return Err(Error::ControlTasksSaturated {
                max: self.config.max_in_flight_calls,
            });
        }

        let write = Arc::clone(&self.write);
        let write_timeout = self.config.control_write_timeout;
        let body = postcard::to_allocvec("inbound request stream buffer exceeded")
            .unwrap_or_else(|_| Vec::new());

        self.control_tasks.spawn(async move {
            let message = WireMessage::StreamError {
                id,
                code: StatusCode::from(PredefinedCode::BadInput),
                body,
            };
            let result = match timeout(write_timeout, write.lock()).await {
                Ok(mut write) => write_message(&mut *write, &message).await,
                Err(_) => Err(Error::ControlWriteLockTimeout {
                    timeout: write_timeout,
                }),
            };

            ControlCompletion { id, result }
        });

        Ok(())
    }

    fn finish_control_task(
        &mut self,
        result: std::result::Result<ControlCompletion, tokio::task::JoinError>,
    ) -> Result<()> {
        let completion = result.map_err(|error| Error::ControlTask(error.to_string()))?;

        completion.result?;

        if self
            .calls
            .get(&completion.id)
            .is_some_and(|slot| slot.tombstone)
        {
            self.calls.remove(&completion.id);
        }

        Ok(())
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
                    if self.calls.get(&done).is_some_and(|slot| !slot.tombstone) {
                        self.calls.remove(&done);
                    }
                }

                Some(result) = self.control_tasks.join_next(), if !self.control_tasks.is_empty() => {
                    if let Err(error) = self.finish_control_task(result) {
                        self.cancel_all();

                        return Err(error);
                    }
                }

                msg = self.reader.read_message() => {
                    match msg {
                        Ok(WireMessage::Request(req)) => {
                            let id = req.id;

                            if self.calls.contains_key(&id) {
                                self.cancel_all();

                                return Err(Error::DuplicateCallId { id });
                            }

                            if self.calls.len() >= self.config.max_in_flight_calls {
                                self.cancel_all();

                                return Err(Error::TooManyCalls {
                                    max: self.config.max_in_flight_calls,
                                });
                            }

                            tracing::Span::current()
                                .record("id", id)
                                .record("path", tracing::field::display(&req.path));

                            debug!("call received");

                            let cancel = CancellationToken::new();
                            let active = Arc::new(AtomicBool::new(true));
                            let requests = if req.streaming_input {
                                let (tx, rx) = mpsc::channel(INBOUND_BUFFER);

                                self.calls.insert(id, CallSlot {
                                    inbound: Some(tx),
                                    cancel: cancel.clone(),
                                    active: Arc::clone(&active),
                                    tombstone: false,
                                });

                                Some(rx)
                            } else {
                                self.calls.insert(id, CallSlot {
                                    inbound: None,
                                    cancel: cancel.clone(),
                                    active: Arc::clone(&active),
                                    tombstone: false,
                                });

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
                                active,
                            };

                            return Ok(Some((call, responder)));
                        }

                        Ok(WireMessage::StreamItem { id, payload }) => {
                            let sender = self.calls.get(&id).and_then(|s| s.inbound.clone());

                            if let Some(tx) = sender {
                                match tx.try_send(payload) {
                                    Ok(()) => {}
                                    Err(mpsc::error::TrySendError::Full(_)) => {
                                        warn!(%id, "inbound stream buffer exceeded; terminating call");
                                        if let Err(error) = self.reject_inbound_overflow(id) {
                                            self.cancel_all();

                                            return Err(error);
                                        }
                                    }
                                    Err(mpsc::error::TrySendError::Closed(_)) => {
                                        if let Some(slot) = self.calls.get_mut(&id) {
                                            slot.inbound = None;
                                        }
                                    }
                                }
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
        self.control_tasks.abort_all();
    }
}

impl<W> Respond for StreamResponder<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    #[instrument(level = "trace", skip_all, fields(id = self.id))]
    async fn respond(self, outcome: CallResult) -> Result<()> {
        trace!("writing response");

        if !self.active.swap(false, Ordering::AcqRel) {
            return Err(Error::Closed);
        }

        let msg = WireMessage::Response(WireResponse::new(self.id, outcome));

        let result = {
            let mut write = self.write.lock().await;

            write_message(&mut *write, &msg).await
        };

        let _ = self.completions_tx.send(self.id);

        result?;

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
            active: self.active,
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

        let mut write = self.write.lock().await;

        if !self.active.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }

        write_message(
            &mut *write,
            &WireMessage::StreamItem {
                id: self.id,
                payload: item,
            },
        )
        .await
    }

    #[instrument(level = "trace", skip_all, fields(id = self.id))]
    async fn error(self, code: StatusCode, body: Vec<u8>) -> Result<()> {
        trace!("writing stream error");

        if !self.active.swap(false, Ordering::AcqRel) {
            return Err(Error::Closed);
        }

        let result = self
            .write_frame(&WireMessage::StreamError {
                id: self.id,
                code,
                body,
            })
            .await;

        let _ = self.completions_tx.send(self.id);

        result
    }

    #[instrument(level = "trace", skip_all, fields(id = self.id))]
    async fn finish(self) -> Result<()> {
        trace!("writing stream end");

        if !self.active.swap(false, Ordering::AcqRel) {
            return Err(Error::Closed);
        }

        let result = self
            .write_frame(&WireMessage::StreamEnd { id: self.id })
            .await;

        let _ = self.completions_tx.send(self.id);

        result
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

#[cfg(test)]
mod tests;
