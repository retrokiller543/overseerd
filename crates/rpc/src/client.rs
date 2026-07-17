//! The RPC protocol's client: a byte-stream carry plus implementations of the
//! [`overseerd_client`] capability traits.
//!
//! [`StreamClientTransport`] is the public transport (TCP/Unix); the call carry
//! (`RpcCall`/`RpcSink`/`RpcSource`, the demux read loop, the `Reply` frames) is RPC-internal.
//! On top of it, the transport implements [`Encodes`]/[`Decodes`] (postcard) and the
//! [`Unary`]/[`ServerStreaming`]/[`ClientStreaming`]/[`BidiStreaming`] capabilities — RPC
//! supports all four. The generated client is generic over those capabilities, so it never
//! names anything in this module.

use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::task::{Context, Poll};
use std::time::Duration;

use futures::{Stream, StreamExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use overseerd_client::{
    BidiStreaming, ClientError, ClientStreaming, ErrorBody, ServerStreaming, StreamArg, Transport,
    Unary, retype,
};
use overseerd_transport::protocol::{
    WireMessage, WireRequest, WireResponse,
    codec::{FrameConfig, MessageReader},
};
use overseerd_transport::{CallId, CodecError, Decodes, Encodes, Error, StatusCode, WireOutcome};

/// Outbound frames buffered per call before the read loop backpressures.
const REPLY_BUFFER: usize = 32;

/// Outbound frames queued for the single-owner writer task. Data and cancellation share one FIFO
/// so a terminal cancellation can never overtake an already accepted request/item frame.
const WRITE_BUFFER: usize = 32;

/// Maximum time the final transport drop gives accepted frames to drain before aborting the
/// writer and dropping its socket.
const CLIENT_DRAIN_TIMEOUT: Duration = Duration::from_millis(250);

/// The stable local error returned when a peer outruns a streaming response consumer.
const REPLY_OVERFLOW_ERROR: &str = "local RPC reply buffer exceeded";

/// A demuxed frame belonging to one in-flight call (the `CallId` already stripped).
enum Reply {
    Response(WireOutcome),
    Item(Vec<u8>),
    End,
    Error { code: StatusCode, body: Vec<u8> },
    Overflow,
}

/// The terminal lane is independent of the bounded item lane. This lets the read loop end a
/// call without awaiting capacity, including when all item slots are occupied.
struct CallRoute {
    items: mpsc::Sender<Vec<u8>>,
    terminal: oneshot::Sender<Reply>,
    active: Arc<AtomicBool>,
}

/// The per-call routing table. Shared between `open` (registration) and the read loop
/// (demuxing); a synchronous mutex so it can also be cleared from `Drop`.
type CallTable = Arc<StdMutex<HashMap<CallId, CallRoute>>>;

/// One pre-serialized FIFO writer command. Ordinary callers await an acknowledgement; Drop paths
/// submit terminal frames with `try_send` and poison the connection if the bounded queue is full.
enum WriteCommand {
    Data {
        frame: Vec<u8>,
        written: oneshot::Sender<Result<(), Error>>,
    },
    Control(Vec<u8>),
}

/// Shared connection liveness. Either actor may poison the connection and wake the other.
struct ConnectionState {
    closed: AtomicBool,
    shutdown: CancellationToken,
    drain: CancellationToken,
}

impl ConnectionState {
    fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            shutdown: CancellationToken::new(),
            drain: CancellationToken::new(),
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.shutdown.cancel();
    }

    fn start_drain(&self) {
        self.closed.store(true, Ordering::Release);
        self.drain.cancel();
    }
}

/// Cloneable handle to the task that exclusively owns the byte-stream writer.
#[derive(Clone)]
struct Writer {
    queue: mpsc::Sender<WriteCommand>,
    state: Arc<ConnectionState>,
}

impl Writer {
    async fn write_message(&self, message: &WireMessage) -> Result<(), ClientError<StatusCode>> {
        let frame = serialize_frame(message)?;

        self.write_frame(frame).await
    }

    async fn write_frame(&self, frame: Vec<u8>) -> Result<(), ClientError<StatusCode>> {
        if self.state.is_closed() {
            return Err(ClientError::ConnectionClosed);
        }

        let (written, wait_written) = oneshot::channel();
        self.queue
            .send(WriteCommand::Data { frame, written })
            .await
            .map_err(|_| ClientError::ConnectionClosed)?;

        wait_written
            .await
            .map_err(|_| ClientError::ConnectionClosed)??;

        Ok(())
    }

    /// Enqueues a pre-serialized cancellation without blocking in `Drop`. Failure means the
    /// writer actor is already gone, so poison the shared connection state.
    fn cancel(&self, frame: Vec<u8>) {
        if self.state.is_closed() {
            return;
        }

        if self.queue.try_send(WriteCommand::Control(frame)).is_err() {
            self.state.close();
        }
    }

    fn close(&self) {
        self.state.close();
    }
}

/// Owns a newly registered route across the cancellation points in `open`. Until ownership is
/// transferred to `RpcCall`, dropping the future removes the route and submits a terminal cancel.
struct OpenGuard {
    id: CallId,
    writer: Writer,
    calls: CallTable,
    active: Arc<AtomicBool>,
    cancel: Option<Vec<u8>>,
    armed: bool,
}

impl OpenGuard {
    fn into_call<W>(
        mut self,
        items: mpsc::Receiver<Vec<u8>>,
        terminal: oneshot::Receiver<Reply>,
    ) -> RpcCall<W> {
        self.armed = false;

        RpcCall {
            id: self.id,
            writer: self.writer.clone(),
            calls: Arc::clone(&self.calls),
            items,
            terminal: Some(terminal),
            active: Arc::clone(&self.active),
            cancel: self.cancel.take(),
            _write: PhantomData,
        }
    }
}

impl Drop for OpenGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        if let Ok(mut calls) = self.calls.lock() {
            calls.remove(&self.id);
        }

        if self.active.swap(false, Ordering::AcqRel)
            && let Some(cancel) = self.cancel.take()
        {
            self.writer.cancel(cancel);
        }
    }
}

/// Owns both connection actors while at least one transport handle remains.
struct ClientTasks {
    read: JoinHandle<()>,
    write: Option<JoinHandle<()>>,
    _writer: Writer,
    calls: CallTable,
}

impl Drop for ClientTasks {
    fn drop(&mut self) {
        // `RpcResponses` drops its source before its transport, so a last-handle cancellation is
        // already queued. Stop admission, ask the writer to drain accepted data/control frames,
        // then enforce a hard deadline so a stalled socket can never leave a detached task alive.
        self.read.abort();
        self._writer.state.start_drain();

        if let Ok(mut calls) = self.calls.lock() {
            calls.clear();
        }

        let Some(mut write) = self.write.take() else {
            return;
        };
        let state = Arc::clone(&self._writer.state);

        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if tokio::time::timeout(CLIENT_DRAIN_TIMEOUT, &mut write)
                    .await
                    .is_err()
                {
                    state.close();
                    write.abort();
                    let _ = write.await;
                }
            });
        } else {
            state.close();
            write.abort();
        }
    }
}

/// An RPC client transport over any reliable, ordered byte stream (TCP, Unix).
///
/// One background task owns the read half and demuxes `Response`/`StreamItem`/`StreamEnd`/
/// `StreamError` frames by `CallId` into per-call channels. A single writer actor owns the write
/// half and finishes each accepted frame even if its caller is cancelled. Cheaply cloneable (all
/// shared state is `Arc`): a response stream holds a clone to decode its items, and the actors
/// remain available until the last clone drops.
pub struct StreamClientTransport<W> {
    writer: Writer,
    next_id: Arc<AtomicU64>,
    calls: CallTable,
    _tasks: Arc<ClientTasks>,
    _write: PhantomData<fn() -> W>,
}

// Manual `Clone` — all state is `Arc`, so cloning is independent of `W` (a derived impl
// would wrongly demand `W: Clone`).
impl<W> Clone for StreamClientTransport<W> {
    fn clone(&self) -> Self {
        Self {
            writer: self.writer.clone(),
            next_id: Arc::clone(&self.next_id),
            calls: Arc::clone(&self.calls),
            _tasks: Arc::clone(&self._tasks),
            _write: PhantomData,
        }
    }
}

impl<W> StreamClientTransport<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    /// Splits ownership: `read` moves into the demux task, `write` stays shared.
    pub fn new<R>(read: R, write: W) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        Self::with_frame_config(read, write, FrameConfig::default())
    }

    /// Builds a client with explicit frame-size and idle-read limits.
    pub fn with_frame_config<R>(read: R, write: W, frame_config: FrameConfig) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        let calls: CallTable = Arc::new(StdMutex::new(HashMap::new()));
        let state = Arc::new(ConnectionState::new());
        let (queue_tx, queue_rx) = mpsc::channel(WRITE_BUFFER);
        let writer = Writer {
            queue: queue_tx,
            state: Arc::clone(&state),
        };
        let reader = MessageReader::with_config(read, frame_config);
        let read_task = tokio::spawn(read_loop(reader, Arc::clone(&calls), writer.clone()));
        let write_task = tokio::spawn(write_loop(write, queue_rx, Arc::clone(&state)));
        let tasks = Arc::new(ClientTasks {
            read: read_task,
            write: Some(write_task),
            _writer: writer.clone(),
            calls: Arc::clone(&calls),
        });

        Self {
            writer,
            next_id: Arc::new(AtomicU64::new(1)),
            calls,
            _tasks: tasks,
            _write: PhantomData,
        }
    }

    /// Opens a call: assigns a `CallId`, registers its reply channel, and writes the opening
    /// `Request` frame. `streaming_input` marks a client/bidi call whose request body streams.
    async fn open(
        &self,
        path: &str,
        streaming_input: bool,
        payload: Vec<u8>,
    ) -> Result<RpcCall<W>, ClientError<StatusCode>> {
        if self.writer.state.is_closed() {
            return Err(ClientError::ConnectionClosed);
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (items_tx, items_rx) = mpsc::channel(REPLY_BUFFER);
        let (terminal_tx, terminal_rx) = oneshot::channel();
        let active = Arc::new(AtomicBool::new(true));
        let request = WireMessage::Request(WireRequest {
            id,
            path: path.to_string(),
            payload,
            streaming_input,
        });
        let request = serialize_frame(&request)?;
        let cancel = serialize_frame(&WireMessage::StreamCancel { id })?;

        // Register before writing so a fast reply can't race an absent entry.
        self.calls.lock().unwrap().insert(
            id,
            CallRoute {
                items: items_tx,
                terminal: terminal_tx,
                active: Arc::clone(&active),
            },
        );
        let guard = OpenGuard {
            id,
            writer: self.writer.clone(),
            calls: Arc::clone(&self.calls),
            active,
            cancel: Some(cancel),
            armed: true,
        };

        if self.writer.state.is_closed() {
            return Err(ClientError::ConnectionClosed);
        }

        self.writer.write_frame(request).await?;

        Ok(guard.into_call(items_rx, terminal_rx))
    }
}

/// One in-flight call, split into a [`RpcSink`] (shared write half) and a [`RpcSource`]
/// (reply receiver) so the two directions run independently.
struct RpcCall<W> {
    id: CallId,
    writer: Writer,
    calls: CallTable,
    items: mpsc::Receiver<Vec<u8>>,
    terminal: Option<oneshot::Receiver<Reply>>,
    active: Arc<AtomicBool>,
    cancel: Option<Vec<u8>>,
    _write: PhantomData<fn() -> W>,
}

impl<W> RpcCall<W> {
    fn split(self) -> (RpcSink<W>, RpcSource) {
        (
            RpcSink {
                id: self.id,
                writer: self.writer.clone(),
                _write: PhantomData,
            },
            RpcSource {
                id: self.id,
                calls: self.calls,
                writer: self.writer,
                items: self.items,
                terminal: self.terminal,
                active: self.active,
                cancel: self.cancel,
            },
        )
    }
}

/// The send half: submits pre-serialized inbound frames to the writer actor.
struct RpcSink<W> {
    id: CallId,
    writer: Writer,
    _write: PhantomData<fn() -> W>,
}

impl<W> RpcSink<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    async fn write_frame(&self, msg: &WireMessage) -> Result<(), ClientError<StatusCode>> {
        self.writer.write_message(msg).await
    }

    async fn send(&mut self, payload: Vec<u8>) -> Result<(), ClientError<StatusCode>> {
        self.write_frame(&WireMessage::StreamItem {
            id: self.id,
            payload,
        })
        .await
    }

    async fn finish(&mut self) -> Result<(), ClientError<StatusCode>> {
        self.write_frame(&WireMessage::StreamEnd { id: self.id })
            .await
    }
}

/// The receive half: pulls demuxed replies, and on drop removes the call's entry from the
/// routing table and asks the peer to cancel unfinished work.
struct RpcSource {
    id: CallId,
    calls: CallTable,
    writer: Writer,
    items: mpsc::Receiver<Vec<u8>>,
    terminal: Option<oneshot::Receiver<Reply>>,
    active: Arc<AtomicBool>,
    cancel: Option<Vec<u8>>,
}

impl RpcSource {
    async fn recv(&mut self) -> Option<Reply> {
        if let Some(item) = self.items.recv().await {
            return Some(Reply::Item(item));
        }

        let terminal = self.terminal.take()?;

        terminal.await.ok()
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<Reply>> {
        match self.items.poll_recv(cx) {
            Poll::Ready(Some(item)) => return Poll::Ready(Some(Reply::Item(item))),
            Poll::Ready(None) | Poll::Pending => {}
        }

        let Some(terminal) = &mut self.terminal else {
            return Poll::Ready(None);
        };

        match Future::poll(Pin::new(terminal), cx) {
            Poll::Ready(result) => {
                self.terminal = None;

                Poll::Ready(result.ok())
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for RpcSource {
    fn drop(&mut self) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.remove(&self.id);
        }

        if self.active.swap(false, Ordering::AcqRel)
            && let Some(cancel) = self.cancel.take()
        {
            self.writer.cancel(cancel);
        }
    }
}

// ---------------------------------------------------------------------------
// Serialization: RPC uses postcard for every serde message type.
// ---------------------------------------------------------------------------

impl<W, T> Encodes<T> for StreamClientTransport<W>
where
    W: Send + 'static,
    T: Serialize,
{
    fn encode(&self, value: T) -> Result<Vec<u8>, CodecError> {
        postcard::to_allocvec(&value).map_err(|e| CodecError::internal(e.to_string()))
    }
}

impl<W, T> Decodes<T> for StreamClientTransport<W>
where
    W: Send + 'static,
    T: DeserializeOwned,
{
    fn decode(&self, body: Vec<u8>) -> Result<T, CodecError> {
        postcard::from_bytes(&body).map_err(|e| CodecError::bad_input(e.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Capabilities: RPC supports all four call shapes.
// ---------------------------------------------------------------------------

// RPC's status is the packed wire `StatusCode` carried on `WireOutcome::Err`/`StreamError`.
impl<W> Transport for StreamClientTransport<W>
where
    W: Send + 'static,
{
    type Status = StatusCode;
}

impl<W> Unary for StreamClientTransport<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    // RPC carries only a body on the wire, so both envelopes are pure passthroughs — the
    // request is the body, the response is the decoded value — keeping the call identical to a
    // bare `unary(path, body) -> Resp`.
    type Request<B> = B;
    type Response<R> = R;

    async fn unary<B, Resp, E>(
        &self,
        path: &str,
        request: B,
    ) -> Result<Resp, ClientError<StatusCode, E>>
    where
        Self: Encodes<B> + Decodes<Resp>,
        B: Send,
        Resp: Send,
    {
        let payload = self.encode(request).map_err(encode_err)?;
        let call = self.open(path, false, payload).await.map_err(retype)?;
        let (_sink, mut source) = call.split();

        decode_unary(self, source.recv().await)
    }
}

impl<W> ServerStreaming for StreamClientTransport<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    type Responses<Resp, E>
        = RpcResponses<W, Resp, E>
    where
        Self: Decodes<Resp>;

    async fn server_stream<Req, Resp, E>(
        &self,
        path: &str,
        request: Req,
    ) -> Result<Self::Responses<Resp, E>, ClientError<StatusCode, E>>
    where
        Self: Encodes<Req> + Decodes<Resp>,
        Req: Send,
        Resp: Send,
    {
        let payload = self.encode(request).map_err(encode_err)?;
        let call = self.open(path, false, payload).await.map_err(retype)?;
        let (_sink, source) = call.split();

        Ok(RpcResponses::new(self.clone(), source))
    }
}

impl<W> ClientStreaming for StreamClientTransport<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    async fn client_stream<Req, Resp, E, I>(
        &self,
        path: &str,
        requests: I,
    ) -> Result<Resp, ClientError<StatusCode, E>>
    where
        Self: Encodes<Req> + Decodes<Resp>,
        Req: Send + 'static,
        Resp: Send,
        I: Into<StreamArg<Req>> + Send,
    {
        let call = self.open(path, true, Vec::new()).await.map_err(retype)?;
        let (mut sink, mut source) = call.split();
        let mut input = requests.into().into_inner();

        while let Some(item) = input.next().await {
            let bytes = self.encode(item).map_err(encode_err)?;
            sink.send(bytes).await.map_err(retype)?;
        }

        sink.finish().await.map_err(retype)?;

        decode_unary(self, source.recv().await)
    }
}

impl<W> BidiStreaming for StreamClientTransport<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    type Responses<Resp, E>
        = RpcResponses<W, Resp, E>
    where
        Self: Decodes<Resp>;

    async fn bidi_stream<Req, Resp, E, I>(
        &self,
        path: &str,
        requests: I,
    ) -> Result<Self::Responses<Resp, E>, ClientError<StatusCode, E>>
    where
        Self: Encodes<Req> + Decodes<Resp>,
        Req: Send + 'static,
        Resp: Send,
        I: Into<StreamArg<Req>> + Send,
    {
        let call = self.open(path, true, Vec::new()).await.map_err(retype)?;
        let (mut sink, source) = call.split();
        let mut input = requests.into().into_inner();

        // Pump the request stream on its own task so sending and receiving run concurrently.
        let codec = self.clone();
        let pump = tokio::spawn(async move {
            while let Some(item) = input.next().await {
                let Ok(bytes) = codec.encode(item) else {
                    break;
                };

                if sink.send(bytes).await.is_err() {
                    break;
                }
            }

            let _ = sink.finish().await;
        });

        Ok(RpcResponses::new(self.clone(), source).with_pump(pump))
    }
}

/// The response stream of a server- or bidirectional-streaming RPC call. Decodes each item
/// through the protocol's [`Decodes`] impl, so parsing stays the protocol's job.
pub struct RpcResponses<W, Resp, E> {
    // Keep the source before the transport: fields are dropped in declaration order, and the
    // source must enqueue its cancellation before the final transport handle stops the actors.
    source: RpcSource,
    transport: StreamClientTransport<W>,
    pump: Option<JoinHandle<()>>,
    _marker: PhantomData<fn() -> (Resp, E)>,
}

impl<W, Resp, E> RpcResponses<W, Resp, E> {
    fn new(transport: StreamClientTransport<W>, source: RpcSource) -> Self {
        Self {
            source,
            transport,
            pump: None,
            _marker: PhantomData,
        }
    }

    fn with_pump(mut self, pump: JoinHandle<()>) -> Self {
        self.pump = Some(pump);
        self
    }
}

impl<W, Resp, E> Drop for RpcResponses<W, Resp, E> {
    fn drop(&mut self) {
        if let Some(pump) = &self.pump {
            pump.abort();
        }
    }
}

impl<W, Resp, E> Stream for RpcResponses<W, Resp, E>
where
    StreamClientTransport<W>: Decodes<Resp>,
{
    type Item = Result<Resp, ClientError<StatusCode, E>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        match this.source.poll_recv(cx) {
            Poll::Ready(reply) => Poll::Ready(decode_item(&this.transport, reply)),

            Poll::Pending => Poll::Pending,
        }
    }
}

/// Maps a body encode failure onto a client error.
fn encode_err<E>(e: CodecError) -> ClientError<StatusCode, E> {
    ClientError::Encode(e.message)
}

/// Decodes the single response of a unary or client-streaming call.
fn decode_unary<C, Resp, E>(
    codec: &C,
    reply: Option<Reply>,
) -> Result<Resp, ClientError<StatusCode, E>>
where
    C: Decodes<Resp>,
{
    match reply {
        Some(Reply::Response(WireOutcome::Ok(bytes))) => codec
            .decode(bytes)
            .map_err(|e| ClientError::Decode(e.message)),

        Some(Reply::Response(WireOutcome::Err { code, body }))
        | Some(Reply::Error { code, body }) => Err(ClientError::Remote(ErrorBody::new(code, body))),

        None | Some(Reply::End) => Err(ClientError::ConnectionClosed),

        Some(Reply::Item(_)) => Err(ClientError::Decode(
            "unexpected stream item awaiting unary response".into(),
        )),

        Some(Reply::Overflow) => Err(ClientError::Decode(REPLY_OVERFLOW_ERROR.into())),
    }
}

/// Decodes the next item of a streaming call: `None` at end-of-stream, a terminal error as
/// `Some(Err(..))`.
fn decode_item<C, Resp, E>(
    codec: &C,
    reply: Option<Reply>,
) -> Option<Result<Resp, ClientError<StatusCode, E>>>
where
    C: Decodes<Resp>,
{
    match reply {
        Some(Reply::Item(bytes)) => Some(
            codec
                .decode(bytes)
                .map_err(|e| ClientError::Decode(e.message)),
        ),

        Some(Reply::Error { code, body }) => {
            Some(Err(ClientError::Remote(ErrorBody::new(code, body))))
        }

        Some(Reply::Response(WireOutcome::Err { code, body })) => {
            Some(Err(ClientError::Remote(ErrorBody::new(code, body))))
        }

        Some(Reply::Response(WireOutcome::Ok(_))) => Some(Err(ClientError::Decode(
            "unexpected unary response in stream".into(),
        ))),

        Some(Reply::Overflow) => Some(Err(ClientError::Decode(REPLY_OVERFLOW_ERROR.into()))),

        None | Some(Reply::End) => None,
    }
}

/// Demuxes inbound frames into per-call channels until the stream ends or errors. Terminal
/// replies use a one-shot lane, so delivering them never waits behind a full item buffer.
async fn read_loop<R>(mut reader: MessageReader<R>, calls: CallTable, writer: Writer)
where
    R: AsyncRead + Unpin,
{
    loop {
        let message = tokio::select! {
            _ = writer.state.shutdown.cancelled() => break,
            message = reader.read_message() => message,
        };

        match message {
            Ok(WireMessage::Response(WireResponse { id, outcome })) => {
                complete_call(&calls, id, Reply::Response(outcome));
            }

            Ok(WireMessage::StreamItem { id, payload }) => {
                let sender = calls
                    .lock()
                    .unwrap()
                    .get(&id)
                    .map(|route| route.items.clone());

                if let Some(tx) = sender {
                    match tx.try_send(payload) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            warn!(%id, "reply buffer exceeded; terminating call");

                            if complete_call(&calls, id, Reply::Overflow) {
                                cancel_overflowed_call(&writer, id);
                            }
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            calls.lock().unwrap().remove(&id);
                        }
                    }
                }
            }

            Ok(WireMessage::StreamEnd { id }) => {
                complete_call(&calls, id, Reply::End);
            }

            Ok(WireMessage::StreamError { id, code, body }) => {
                complete_call(&calls, id, Reply::Error { code, body });
            }

            Ok(WireMessage::Request(_)) | Ok(WireMessage::StreamCancel { .. }) => {
                warn!("unexpected server-bound message on client connection");

                break;
            }

            Err(Error::Io(e)) if is_disconnect(&e) => {
                debug!(error = %e, "server disconnected");

                break;
            }

            Err(e) => {
                warn!(error = %e, "client frame read error");

                break;
            }
        }
    }

    writer.close();
    calls.lock().unwrap().clear();
}

/// Removes a call and publishes its terminal state without waiting for bounded capacity.
fn complete_call(calls: &CallTable, id: CallId, reply: Reply) -> bool {
    let route = calls.lock().unwrap().remove(&id);

    let Some(route) = route else {
        return false;
    };

    route.active.store(false, Ordering::Release);
    let _ = route.terminal.send(reply);

    true
}

/// Overflow is a local terminal error and a remote cancellation. If even serialization of the
/// fixed control frame fails, poison the connection rather than leaving remote work orphaned.
fn cancel_overflowed_call(writer: &Writer, id: CallId) {
    match serialize_frame(&WireMessage::StreamCancel { id }) {
        Ok(frame) => writer.cancel(frame),
        Err(_) => writer.close(),
    }
}

/// Serializes the complete length-prefixed frame before handing it to the writer actor.
fn serialize_frame(message: &WireMessage) -> Result<Vec<u8>, Error> {
    let payload =
        postcard::to_allocvec(message).map_err(|error| Error::Serialization(error.to_string()))?;
    let len = u32::try_from(payload.len()).map_err(|_| Error::FrameTooLarge {
        len: payload.len(),
        max: u32::MAX as usize,
    })?;
    let frame_len = payload
        .len()
        .checked_add(size_of::<u32>())
        .ok_or(Error::FrameAllocation { len: payload.len() })?;
    let mut frame = Vec::new();
    frame
        .try_reserve_exact(frame_len)
        .map_err(|_| Error::FrameAllocation { len: frame_len })?;
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(&payload);

    Ok(frame)
}

/// Owns the write half for its entire lifetime. Once a command is accepted, cancellation of the
/// caller waiting for its acknowledgement cannot cancel the underlying frame write.
async fn write_loop<W>(
    mut write: W,
    mut queue: mpsc::Receiver<WriteCommand>,
    state: Arc<ConnectionState>,
) where
    W: AsyncWrite + Unpin,
{
    loop {
        tokio::select! {
            biased;

            _ = state.shutdown.cancelled() => break,

            _ = state.drain.cancelled() => {
                drain_write_queue(&mut write, &mut queue, &state).await;

                break;
            }

            Some(command) = queue.recv() => {
                if execute_write_command(&mut write, command, &state).await.is_err() {
                    state.close();

                    break;
                }
            }

            else => break,
        }
    }

    state.close();
}

/// Stops sender admission and drains only commands accepted before the final transport handle
/// was dropped. The owning `ClientTasks` watchdog forces `shutdown` if any frame stalls.
async fn drain_write_queue<W>(
    write: &mut W,
    queue: &mut mpsc::Receiver<WriteCommand>,
    state: &ConnectionState,
) where
    W: AsyncWrite + Unpin,
{
    queue.close();

    loop {
        tokio::select! {
            biased;

            _ = state.shutdown.cancelled() => break,

            Some(command) = queue.recv() => {
                if execute_write_command(write, command, state).await.is_err() {
                    break;
                }
            }

            else => break,
        }
    }
}

async fn execute_write_command<W>(
    write: &mut W,
    command: WriteCommand,
    state: &ConnectionState,
) -> Result<(), Error>
where
    W: AsyncWrite + Unpin,
{
    match command {
        WriteCommand::Data { frame, written } => {
            let result = write_serialized(write, &frame, state).await;
            let failed = result.is_err();
            let _ = written.send(result);

            if failed {
                warn!("client data-frame write failed");

                Err(Error::Closed)
            } else {
                Ok(())
            }
        }
        WriteCommand::Control(frame) => {
            let result = write_serialized(write, &frame, state).await;

            if let Err(error) = &result {
                warn!(%error, "client control-frame write error");
            }

            result
        }
    }
}

async fn write_serialized<W>(
    write: &mut W,
    frame: &[u8],
    state: &ConnectionState,
) -> Result<(), Error>
where
    W: AsyncWrite + Unpin,
{
    tokio::select! {
        biased;

        _ = state.shutdown.cancelled() => Err(Error::Closed),
        result = write.write_all(frame) => result.map_err(Error::from),
    }
}

/// Distinguishes an orderly server disconnect from a genuine I/O failure.
fn is_disconnect(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
    )
}

/// Connects over TCP, returning a ready [`StreamClientTransport`]. Wrap it in a generated
/// client (`FooClient::new(transport)`).
pub async fn connect_tcp(
    addr: impl tokio::net::ToSocketAddrs,
) -> Result<StreamClientTransport<tokio::net::tcp::OwnedWriteHalf>, ClientError<StatusCode>> {
    let stream = tokio::net::TcpStream::connect(addr)
        .await
        .map_err(|e| ClientError::Transport(Error::Io(e)))?;
    let (read, write) = stream.into_split();

    Ok(StreamClientTransport::new(read, write))
}

/// Connects over a Unix socket, returning a ready [`StreamClientTransport`].
#[cfg(unix)]
pub async fn connect_unix(
    path: impl AsRef<std::path::Path>,
) -> Result<StreamClientTransport<tokio::net::unix::OwnedWriteHalf>, ClientError<StatusCode>> {
    let stream = tokio::net::UnixStream::connect(path)
        .await
        .map_err(|e| ClientError::Transport(Error::Io(e)))?;
    let (read, write) = stream.into_split();

    Ok(StreamClientTransport::new(read, write))
}

#[cfg(test)]
mod tests;
