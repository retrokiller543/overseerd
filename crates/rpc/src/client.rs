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
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::task::{Context, Poll};

use futures::{Stream, StreamExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use overseerd_client::{
    BidiStreaming, ClientError, ClientStreaming, ErrorBody, ServerStreaming, StreamArg, Unary,
    retype,
};
use overseerd_transport::protocol::{
    WireMessage, WireRequest, WireResponse,
    codec::{read_message, write_message},
};
use overseerd_transport::{CallId, CodecError, Decodes, Encodes, Error, StatusCode, WireOutcome};

/// Outbound frames buffered per call before the read loop backpressures.
const REPLY_BUFFER: usize = 32;

/// A demuxed outbound frame belonging to one in-flight call (the `CallId` already stripped).
enum Reply {
    Response(WireOutcome),
    Item(Vec<u8>),
    End,
    Error { code: StatusCode, body: Vec<u8> },
}

/// The per-call routing table: maps a `CallId` to the sender feeding that call's reply
/// channel. Shared between `open` (registration) and the read loop (demuxing); a synchronous
/// mutex so it can also be cleared from `Drop`.
type CallTable = Arc<StdMutex<HashMap<CallId, mpsc::Sender<Reply>>>>;

/// Aborts the demux read task when the last transport handle is dropped.
struct ReadTask(JoinHandle<()>);

impl Drop for ReadTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// An RPC client transport over any reliable, ordered byte stream (TCP, Unix).
///
/// One background task owns the read half and demuxes `Response`/`StreamItem`/`StreamEnd`/
/// `StreamError` frames by `CallId` into per-call channels; the write half is shared behind a
/// mutex, locked only for a single frame write. Cheaply cloneable (all shared state is
/// `Arc`): a response stream holds a clone to decode its items, and the read task is aborted
/// only when the last clone drops.
pub struct StreamClientTransport<W> {
    write: Arc<Mutex<W>>,
    next_id: Arc<AtomicU64>,
    calls: CallTable,
    _read_task: Arc<ReadTask>,
}

// Manual `Clone` — all state is `Arc`, so cloning is independent of `W` (a derived impl
// would wrongly demand `W: Clone`).
impl<W> Clone for StreamClientTransport<W> {
    fn clone(&self) -> Self {
        Self {
            write: Arc::clone(&self.write),
            next_id: Arc::clone(&self.next_id),
            calls: Arc::clone(&self.calls),
            _read_task: Arc::clone(&self._read_task),
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
        let calls: CallTable = Arc::new(StdMutex::new(HashMap::new()));
        let read_task = tokio::spawn(read_loop(read, Arc::clone(&calls)));

        Self {
            write: Arc::new(Mutex::new(write)),
            next_id: Arc::new(AtomicU64::new(1)),
            calls,
            _read_task: Arc::new(ReadTask(read_task)),
        }
    }

    /// Opens a call: assigns a `CallId`, registers its reply channel, and writes the opening
    /// `Request` frame. `streaming_input` marks a client/bidi call whose request body streams.
    async fn open(
        &self,
        path: &str,
        streaming_input: bool,
        payload: Vec<u8>,
    ) -> Result<RpcCall<W>, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(REPLY_BUFFER);
        let request = WireMessage::Request(WireRequest {
            id,
            path: path.to_string(),
            payload,
            streaming_input,
        });

        // Register before writing so a fast reply can't race an absent entry.
        self.calls.lock().unwrap().insert(id, tx);

        {
            let mut write = self.write.lock().await;

            if let Err(e) = write_message(&mut *write, &request).await {
                drop(write);
                self.calls.lock().unwrap().remove(&id);

                return Err(e.into());
            }
        }

        Ok(RpcCall {
            id,
            write: Arc::clone(&self.write),
            calls: Arc::clone(&self.calls),
            replies: rx,
        })
    }
}

/// One in-flight call, split into a [`RpcSink`] (shared write half) and a [`RpcSource`]
/// (reply receiver) so the two directions run independently.
struct RpcCall<W> {
    id: CallId,
    write: Arc<Mutex<W>>,
    calls: CallTable,
    replies: mpsc::Receiver<Reply>,
}

impl<W> RpcCall<W> {
    fn split(self) -> (RpcSink<W>, RpcSource) {
        (
            RpcSink {
                id: self.id,
                write: self.write,
                calls: Arc::clone(&self.calls),
            },
            RpcSource {
                id: self.id,
                replies: self.replies,
                calls: self.calls,
            },
        )
    }
}

/// The send half: writes inbound frames under the shared write lock.
struct RpcSink<W> {
    id: CallId,
    write: Arc<Mutex<W>>,
    calls: CallTable,
}

impl<W> RpcSink<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    async fn write_frame(&self, msg: &WireMessage) -> Result<(), ClientError> {
        let mut write = self.write.lock().await;

        write_message(&mut *write, msg).await.map_err(Into::into)
    }

    async fn send(&mut self, payload: Vec<u8>) -> Result<(), ClientError> {
        self.write_frame(&WireMessage::StreamItem {
            id: self.id,
            payload,
        })
        .await
    }

    async fn finish(&mut self) -> Result<(), ClientError> {
        self.write_frame(&WireMessage::StreamEnd { id: self.id })
            .await
    }

    #[allow(dead_code)]
    async fn cancel(self) -> Result<(), ClientError> {
        self.write_frame(&WireMessage::StreamCancel { id: self.id })
            .await?;

        self.calls.lock().unwrap().remove(&self.id);

        Ok(())
    }
}

/// The receive half: pulls demuxed replies, and on drop removes the call's entry from the
/// routing table so the read loop stops demuxing into a dead channel.
struct RpcSource {
    id: CallId,
    calls: CallTable,
    replies: mpsc::Receiver<Reply>,
}

impl RpcSource {
    async fn recv(&mut self) -> Option<Reply> {
        self.replies.recv().await
    }
}

impl Drop for RpcSource {
    fn drop(&mut self) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.remove(&self.id);
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

impl<W> Unary for StreamClientTransport<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    async fn unary<Req, Resp, E>(&self, path: &str, request: Req) -> Result<Resp, ClientError<E>>
    where
        Self: Encodes<Req> + Decodes<Resp>,
        Req: Send,
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
    ) -> Result<Self::Responses<Resp, E>, ClientError<E>>
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
    ) -> Result<Resp, ClientError<E>>
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
    ) -> Result<Self::Responses<Resp, E>, ClientError<E>>
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
        tokio::spawn(async move {
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

        Ok(RpcResponses::new(self.clone(), source))
    }
}

/// The response stream of a server- or bidirectional-streaming RPC call. Decodes each item
/// through the protocol's [`Decodes`] impl, so parsing stays the protocol's job.
pub struct RpcResponses<W, Resp, E> {
    transport: StreamClientTransport<W>,
    source: RpcSource,
    _marker: PhantomData<fn() -> (Resp, E)>,
}

impl<W, Resp, E> RpcResponses<W, Resp, E> {
    fn new(transport: StreamClientTransport<W>, source: RpcSource) -> Self {
        Self {
            transport,
            source,
            _marker: PhantomData,
        }
    }
}

impl<W, Resp, E> Stream for RpcResponses<W, Resp, E>
where
    StreamClientTransport<W>: Decodes<Resp>,
{
    type Item = Result<Resp, ClientError<E>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        match this.source.replies.poll_recv(cx) {
            Poll::Ready(reply) => Poll::Ready(decode_item(&this.transport, reply)),

            Poll::Pending => Poll::Pending,
        }
    }
}

/// Maps a body encode failure onto a client error.
fn encode_err<E>(e: CodecError) -> ClientError<E> {
    ClientError::Encode(e.message)
}

/// Decodes the single response of a unary or client-streaming call.
fn decode_unary<C, Resp, E>(codec: &C, reply: Option<Reply>) -> Result<Resp, ClientError<E>>
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
    }
}

/// Decodes the next item of a streaming call: `None` at end-of-stream, a terminal error as
/// `Some(Err(..))`.
fn decode_item<C, Resp, E>(codec: &C, reply: Option<Reply>) -> Option<Result<Resp, ClientError<E>>>
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

        Some(Reply::Response(_)) => Some(Err(ClientError::Decode(
            "unexpected unary response in stream".into(),
        ))),

        None | Some(Reply::End) => None,
    }
}

/// Demuxes inbound frames into per-call channels until the stream ends or errors. On exit the
/// call table is cleared, dropping every reply sender so outstanding calls observe a closed
/// channel and resolve to `ConnectionClosed`.
async fn read_loop<R>(mut read: R, calls: CallTable)
where
    R: AsyncRead + Unpin,
{
    loop {
        let message = read_message(&mut read).await;

        match message {
            Ok(WireMessage::Response(WireResponse { id, outcome })) => {
                let sender = calls.lock().unwrap().remove(&id);

                if let Some(tx) = sender {
                    let _ = tx.send(Reply::Response(outcome)).await;
                }
            }

            Ok(WireMessage::StreamItem { id, payload }) => {
                let sender = calls.lock().unwrap().get(&id).cloned();

                if let Some(tx) = sender {
                    let _ = tx.send(Reply::Item(payload)).await;
                }
            }

            Ok(WireMessage::StreamEnd { id }) => {
                let sender = calls.lock().unwrap().remove(&id);

                if let Some(tx) = sender {
                    let _ = tx.send(Reply::End).await;
                }
            }

            Ok(WireMessage::StreamError { id, code, body }) => {
                let sender = calls.lock().unwrap().remove(&id);

                if let Some(tx) = sender {
                    let _ = tx.send(Reply::Error { code, body }).await;
                }
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

    calls.lock().unwrap().clear();
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
) -> Result<StreamClientTransport<tokio::net::tcp::OwnedWriteHalf>, ClientError> {
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
) -> Result<StreamClientTransport<tokio::net::unix::OwnedWriteHalf>, ClientError> {
    let stream = tokio::net::UnixStream::connect(path)
        .await
        .map_err(|e| ClientError::Transport(Error::Io(e)))?;
    let (read, write) = stream.into_split();

    Ok(StreamClientTransport::new(read, write))
}
