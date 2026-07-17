use std::{
    collections::{HashMap, VecDeque},
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

/// Poll interval used only while a half-closed inbound stream still has buffered items. Retaining
/// its sender until capacity returns lets byte accounting observe consumption before delivering
/// end-of-stream through the public `Receiver<Vec<u8>>` API.
const INBOUND_END_RECONCILE_INTERVAL: Duration = Duration::from_millis(10);

/// Default aggregate bytes buffered across request streams on one connection.
pub const DEFAULT_MAX_INBOUND_BYTES_PER_CONNECTION: usize = 64 * 1024 * 1024;

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
    max_inbound_bytes_per_call: usize,
    max_inbound_bytes_per_connection: usize,
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
            max_inbound_bytes_per_call: frame.max_frame_len(),
            max_inbound_bytes_per_connection: DEFAULT_MAX_INBOUND_BYTES_PER_CONNECTION
                .max(frame.max_frame_len()),
        }
    }

    /// Configures aggregate byte budgets in addition to the fixed item-count channel bound.
    pub fn with_inbound_byte_limits(mut self, per_call: usize, per_connection: usize) -> Self {
        assert!(per_call > 0, "per-call inbound byte limit must be non-zero");
        assert!(
            per_connection >= per_call,
            "connection inbound byte limit must cover at least one call budget"
        );
        self.max_inbound_bytes_per_call = per_call;
        self.max_inbound_bytes_per_connection = per_connection;
        self
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

    pub fn max_inbound_bytes_per_call(self) -> usize {
        self.max_inbound_bytes_per_call
    }

    pub fn max_inbound_bytes_per_connection(self) -> usize {
        self.max_inbound_bytes_per_connection
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
    /// Payload sizes in FIFO order, reconciled against the sender's available capacity whenever
    /// the connection handles another frame for this call.
    inbound_sizes: VecDeque<usize>,
    inbound_bytes: usize,
    inbound_ending: bool,
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

struct CallCompletion {
    id: CallId,
    active: Arc<AtomicBool>,
}

/// Shared logical connection health. Any failed or cancelled frame write poisons the byte
/// stream: waiting writers stop before acquiring/reusing it and the read loop wakes promptly.
struct ConnectionHealth {
    closed: AtomicBool,
    shutdown: CancellationToken,
}

impl ConnectionHealth {
    fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            shutdown: CancellationToken::new(),
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.shutdown.cancel();
        }
    }
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
    completions_tx: mpsc::UnboundedSender<CallCompletion>,
    completions_rx: mpsc::UnboundedReceiver<CallCompletion>,
    control_tasks: JoinSet<ControlCompletion>,
    health: Arc<ConnectionHealth>,
    inbound_bytes: usize,
}

/// Responds to one inbound call on a stream connection. Owns the call's wire
/// id and a shared handle to the connection's write half. Becomes a
/// [`StreamSink`] for streaming responses via [`RespondStream`].
pub struct StreamResponder<W> {
    write: Arc<Mutex<W>>,
    id: CallId,
    completions_tx: mpsc::UnboundedSender<CallCompletion>,
    active: Arc<AtomicBool>,
    health: Arc<ConnectionHealth>,
}

/// The outbound sink for a streaming call, writing `StreamItem`/`StreamEnd`/
/// `StreamError` frames tagged with the call's id.
pub struct StreamSink<W> {
    write: Arc<Mutex<W>>,
    id: CallId,
    completions_tx: mpsc::UnboundedSender<CallCompletion>,
    active: Arc<AtomicBool>,
    health: Arc<ConnectionHealth>,
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
            health: Arc::new(ConnectionHealth::new()),
            inbound_bytes: 0,
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

    fn close(&self) {
        self.health.close();
        self.cancel_all();
    }

    fn reconcile_inbound_bytes(&mut self, id: CallId) {
        let released = {
            let Some(slot) = self.calls.get_mut(&id) else {
                return;
            };
            let Some(inbound) = &slot.inbound else {
                return;
            };
            let queued_items = INBOUND_BUFFER.saturating_sub(inbound.capacity());
            let mut released = 0;

            while slot.inbound_sizes.len() > queued_items {
                if let Some(len) = slot.inbound_sizes.pop_front() {
                    released += len;
                }
            }
            slot.inbound_bytes = slot.inbound_bytes.saturating_sub(released);

            released
        };

        self.inbound_bytes = self.inbound_bytes.saturating_sub(released);
    }

    fn reconcile_all_inbound_bytes(&mut self) {
        let mut released = 0;

        for slot in self.calls.values_mut() {
            let Some(inbound) = &slot.inbound else {
                continue;
            };
            let queued_items = INBOUND_BUFFER.saturating_sub(inbound.capacity());

            while slot.inbound_sizes.len() > queued_items {
                if let Some(len) = slot.inbound_sizes.pop_front() {
                    released += len;
                    slot.inbound_bytes = slot.inbound_bytes.saturating_sub(len);
                }
            }
        }

        self.inbound_bytes = self.inbound_bytes.saturating_sub(released);
    }

    fn remove_call(&mut self, id: CallId) -> Option<CallSlot> {
        let slot = self.calls.remove(&id)?;
        self.inbound_bytes = self.inbound_bytes.saturating_sub(slot.inbound_bytes);

        Some(slot)
    }

    fn discard_closed_inbound(&mut self, id: CallId) {
        let released = {
            let Some(slot) = self.calls.get_mut(&id) else {
                return;
            };

            slot.inbound = None;
            slot.inbound_ending = false;
            slot.inbound_sizes.clear();
            std::mem::take(&mut slot.inbound_bytes)
        };
        self.inbound_bytes = self.inbound_bytes.saturating_sub(released);
    }

    fn finalize_consumed_inbound_ends(&mut self) {
        self.reconcile_all_inbound_bytes();

        for slot in self.calls.values_mut() {
            if slot.inbound_ending && slot.inbound_sizes.is_empty() {
                slot.inbound = None;
                slot.inbound_ending = false;
            }
        }
    }
}

impl<R, W> StreamConnection<R, W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    fn reject_inbound_overflow(&mut self, id: CallId) -> Result<()> {
        self.reconcile_inbound_bytes(id);
        let released = {
            let Some(slot) = self.calls.get_mut(&id) else {
                return Ok(());
            };

            slot.cancel.cancel();
            slot.inbound = None;
            slot.inbound_ending = false;
            slot.inbound_sizes.clear();
            let released = std::mem::take(&mut slot.inbound_bytes);

            let won_terminal = slot.active.swap(false, Ordering::AcqRel);
            if won_terminal {
                slot.tombstone = true;
            }

            (released, won_terminal)
        };
        self.inbound_bytes = self.inbound_bytes.saturating_sub(released.0);
        if !released.1 {
            return Ok(());
        }

        while let Some(result) = self.control_tasks.try_join_next() {
            self.finish_control_task(result)?;
        }

        if self.control_tasks.len() >= self.config.max_in_flight_calls {
            self.close();

            return Err(Error::ControlTasksSaturated {
                max: self.config.max_in_flight_calls,
            });
        }

        let write = Arc::clone(&self.write);
        let health = Arc::clone(&self.health);
        let write_timeout = self.config.control_write_timeout;
        let body = postcard::to_allocvec("inbound request stream buffer exceeded")
            .unwrap_or_else(|_| Vec::new());

        self.control_tasks.spawn(async move {
            let message = WireMessage::StreamError {
                id,
                code: StatusCode::from(PredefinedCode::BadInput),
                body,
            };
            let result = match timeout(
                write_timeout,
                write_connection_frame(&write, &health, &message),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    // The timed future may have written a prefix or part of its body. Poison the
                    // stream immediately; no subsequent writer may reuse those bytes.
                    health.close();
                    Err(Error::ControlWriteTimeout {
                        timeout: write_timeout,
                    })
                }
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
            self.remove_call(completion.id);
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
                    if self.calls.get(&done.id).is_some_and(|slot| {
                        !slot.tombstone && Arc::ptr_eq(&slot.active, &done.active)
                    }) {
                        self.remove_call(done.id);
                    }
                }

                Some(result) = self.control_tasks.join_next(), if !self.control_tasks.is_empty() => {
                    if let Err(error) = self.finish_control_task(result) {
                        self.close();

                        return Err(error);
                    }
                }

                _ = self.health.shutdown.cancelled() => {
                    self.close();

                    return Err(Error::Closed);
                }

                _ = tokio::time::sleep(INBOUND_END_RECONCILE_INTERVAL),
                    if self.calls.values().any(|slot| slot.inbound_ending) =>
                {
                    self.finalize_consumed_inbound_ends();
                }

                msg = self.reader.read_message() => {
                    match msg {
                        Ok(WireMessage::Request(req)) => {
                            let id = req.id;

                            if self.calls.contains_key(&id) {
                                self.close();

                                return Err(Error::DuplicateCallId { id });
                            }

                            if self.calls.len() >= self.config.max_in_flight_calls {
                                self.close();

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
                                    inbound_sizes: VecDeque::new(),
                                    inbound_bytes: 0,
                                    inbound_ending: false,
                                    cancel: cancel.clone(),
                                    active: Arc::clone(&active),
                                    tombstone: false,
                                });

                                Some(rx)
                            } else {
                                self.calls.insert(id, CallSlot {
                                    inbound: None,
                                    inbound_sizes: VecDeque::new(),
                                    inbound_bytes: 0,
                                    inbound_ending: false,
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
                                health: Arc::clone(&self.health),
                            };

                            return Ok(Some((call, responder)));
                        }

                        Ok(WireMessage::StreamItem { id, payload }) => {
                            // A quiet call may have been consumed since its last frame. Reconcile
                            // every bounded sender before enforcing the connection-wide budget.
                            self.reconcile_all_inbound_bytes();
                            let payload_len = payload.len();
                            let route = self.calls.get(&id).and_then(|slot| {
                                if slot.inbound_ending {
                                    return None;
                                }
                                let call_bytes = slot.inbound_bytes.checked_add(payload_len)?;
                                let connection_bytes = self.inbound_bytes.checked_add(payload_len)?;

                                if call_bytes > self.config.max_inbound_bytes_per_call
                                    || connection_bytes
                                        > self.config.max_inbound_bytes_per_connection
                                {
                                    return None;
                                }

                                Some((slot.inbound.as_ref()?.clone(), call_bytes, connection_bytes))
                            });

                            if let Some((tx, call_bytes, connection_bytes)) = route {
                                match tx.try_send(payload) {
                                    Ok(()) => {
                                        if let Some(slot) = self.calls.get_mut(&id) {
                                            slot.inbound_sizes.push_back(payload_len);
                                            slot.inbound_bytes = call_bytes;
                                            self.inbound_bytes = connection_bytes;
                                        }
                                    }
                                    Err(mpsc::error::TrySendError::Full(_)) => {
                                        warn!(%id, "inbound stream buffer exceeded; terminating call");
                                        if let Err(error) = self.reject_inbound_overflow(id) {
                                            self.close();

                                            return Err(error);
                                        }
                                    }
                                    Err(mpsc::error::TrySendError::Closed(_)) => {
                                        self.discard_closed_inbound(id);
                                    }
                                }
                            } else if self
                                .calls
                                .get(&id)
                                .is_some_and(|slot| slot.inbound.is_some() && !slot.inbound_ending)
                            {
                                warn!(%id, "inbound stream byte budget exceeded; terminating call");
                                if let Err(error) = self.reject_inbound_overflow(id) {
                                    self.close();

                                    return Err(error);
                                }
                            }
                        }

                        Ok(WireMessage::StreamEnd { id }) => {
                            self.reconcile_inbound_bytes(id);
                            if let Some(slot) = self.calls.get_mut(&id) {
                                if slot.inbound_sizes.is_empty() {
                                    slot.inbound = None;
                                } else {
                                    slot.inbound_ending = true;
                                }
                            }
                        }

                        Ok(WireMessage::StreamCancel { id }) => {
                            if self.calls.get(&id).is_some_and(|slot| slot.tombstone) {
                                continue;
                            }
                            if let Some(mut slot) = self.remove_call(id) {
                                slot.cancel.cancel();
                                slot.inbound = None;
                                slot.active.store(false, Ordering::Release);
                            }
                        }

                        Err(Error::Io(e)) if is_disconnect(&e) => {
                            debug!(error = %e, "peer disconnected");
                            self.close();

                            return Ok(None);
                        }

                        Ok(WireMessage::Response(_))
                        | Ok(WireMessage::StreamError { .. }) => {
                            warn!("unexpected server-bound message from peer");
                            self.close();

                            return Err(Error::UnexpectedMessage);
                        }

                        Err(e) => {
                            warn!(error = %e, "frame read error");
                            self.close();

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
        self.close();
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

        let result = write_connection_frame(&self.write, &self.health, &msg).await;

        let _ = self.completions_tx.send(CallCompletion {
            id: self.id,
            active: Arc::clone(&self.active),
        });

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
            health: self.health,
        }
    }
}

impl<W> StreamSink<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    /// Writes one frame under the shared write lock, held only for the write.
    async fn write_frame(&self, msg: &WireMessage) -> Result<()> {
        write_connection_frame(&self.write, &self.health, msg).await
    }
}

impl<W> ResponseSink for StreamSink<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    #[instrument(level = "trace", skip_all, fields(id = self.id))]
    async fn send(&mut self, item: Vec<u8>) -> Result<()> {
        trace!("writing stream item");

        if !self.active.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }

        let result = write_active_connection_frame(
            &self.write,
            &self.health,
            &self.active,
            &WireMessage::StreamItem {
                id: self.id,
                payload: item,
            },
        )
        .await;

        if result.is_err() {
            self.active.store(false, Ordering::Release);
            let _ = self.completions_tx.send(CallCompletion {
                id: self.id,
                active: Arc::clone(&self.active),
            });
        }

        result
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

        let _ = self.completions_tx.send(CallCompletion {
            id: self.id,
            active: Arc::clone(&self.active),
        });

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

        let _ = self.completions_tx.send(CallCompletion {
            id: self.id,
            active: Arc::clone(&self.active),
        });

        result
    }
}

/// Poisons a connection if a frame-write future is dropped after acquiring the writer but before
/// the frame operation returns. Cancellation can otherwise leave a prefix/body fragment that a
/// later writer would incorrectly append to.
struct FrameWriteGuard<'a> {
    health: &'a ConnectionHealth,
    completed: bool,
}

impl<'a> FrameWriteGuard<'a> {
    fn new(health: &'a ConnectionHealth) -> Self {
        Self {
            health,
            completed: false,
        }
    }

    fn complete(&mut self) {
        self.completed = true;
    }
}

impl Drop for FrameWriteGuard<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.health.close();
        }
    }
}

/// Writes exactly one frame while enforcing the connection poison invariant. A writer checks
/// health both before and after acquiring the mutex, and an already-running write is cancelled
/// when another path closes the connection. Any I/O error makes the poison permanent.
async fn write_connection_frame<W>(
    write: &Arc<Mutex<W>>,
    health: &ConnectionHealth,
    message: &WireMessage,
) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    if health.is_closed() {
        return Err(Error::Closed);
    }

    let mut write = tokio::select! {
        biased;

        _ = health.shutdown.cancelled() => return Err(Error::Closed),
        write = write.lock() => write,
    };

    if health.is_closed() {
        return Err(Error::Closed);
    }

    let mut attempt = FrameWriteGuard::new(health);
    let result = tokio::select! {
        biased;

        _ = health.shutdown.cancelled() => Err(Error::Closed),
        result = write_message(&mut *write, message) => result,
    };
    attempt.complete();

    if result.is_err() {
        health.close();
    }

    result
}

/// Stream items additionally re-check their call's terminal state after acquiring the shared
/// writer. A cancel processed while this sink waits therefore wins without producing output.
async fn write_active_connection_frame<W>(
    write: &Arc<Mutex<W>>,
    health: &ConnectionHealth,
    active: &AtomicBool,
    message: &WireMessage,
) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    if health.is_closed() || !active.load(Ordering::Acquire) {
        return Err(Error::Closed);
    }

    let mut write = tokio::select! {
        biased;

        _ = health.shutdown.cancelled() => return Err(Error::Closed),
        write = write.lock() => write,
    };

    if health.is_closed() || !active.load(Ordering::Acquire) {
        return Err(Error::Closed);
    }

    let mut attempt = FrameWriteGuard::new(health);
    let result = tokio::select! {
        biased;

        _ = health.shutdown.cancelled() => Err(Error::Closed),
        result = write_message(&mut *write, message) => result,
    };
    attempt.complete();

    if result.is_err() {
        health.close();
    }

    result
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
