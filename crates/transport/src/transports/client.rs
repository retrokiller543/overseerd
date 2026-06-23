use std::future::Future;
use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::Error;
use crate::protocol::WireOutcome;
use crate::status::StatusCode;
use crate::stream_codec::{StreamDecode, StreamEncode};

use super::client_stream::StreamClientTransport;

/// Marker for an error body whose payload type is not known statically (the
/// generic call path). Typed clients substitute the method's declared error type.
#[derive(Debug, Clone, Copy)]
pub struct Raw;

/// `Send`/`Sync`-preserving phantom marker that doesn't bind its type to the
/// owning struct (the markers are only carried for type inference).
type Variance<T> = PhantomData<fn() -> T>;

/// A demuxed outbound frame belonging to one in-flight call. The substrate-level
/// counterpart of the server's response frames, with the `CallId` already stripped.
pub enum Reply {
    Response(WireOutcome),
    Item(Vec<u8>),
    End,
    Error { code: StatusCode, body: Vec<u8> },
}

/// The client mirror of the server's `ErrorResponse`: a status code plus the raw
/// error body bytes. The body is deserialized into `T` lazily and best-effort, so
/// a body the handler serialized as a different type (or as raw, non-postcard
/// bytes) degrades to a failed `deserialize` while `code`/`raw` stay usable.
pub struct ErrorBody<T = Raw> {
    code: StatusCode,
    body: Vec<u8>,
    _marker: Variance<T>,
}

impl<T> ErrorBody<T> {
    pub(crate) fn new(code: StatusCode, body: Vec<u8>) -> Self {
        Self {
            code,
            body,
            _marker: PhantomData,
        }
    }

    pub fn code(&self) -> StatusCode {
        self.code
    }

    pub fn raw(&self) -> &[u8] {
        &self.body
    }

    pub fn into_raw(self) -> Vec<u8> {
        self.body
    }

    /// Re-types the body marker without touching the bytes, e.g. to attach a
    /// known body type to an otherwise [`Raw`] error.
    pub fn cast<U>(self) -> ErrorBody<U> {
        ErrorBody::new(self.code, self.body)
    }
}

impl<T: DeserializeOwned> ErrorBody<T> {
    /// Attempts to decode the body as `T`. Best-effort — see the type docs.
    pub fn deserialize(&self) -> Result<T, postcard::Error> {
        postcard::from_bytes(&self.body)
    }
}

impl<T> Clone for ErrorBody<T> {
    fn clone(&self) -> Self {
        Self::new(self.code, self.body.clone())
    }
}

impl<T> std::fmt::Debug for ErrorBody<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ErrorBody")
            .field("code", &self.code)
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// Everything that can go wrong on a client call. Generic over the error body
/// type `E` so typed clients surface an [`ErrorBody<E>`] with a ready
/// [`deserialize`](ErrorBody::deserialize); the generic call path leaves it [`Raw`].
///
/// Debug/Display/Error are implemented by hand so no bound is placed on `E`
/// (it lives only as a phantom marker inside [`ErrorBody`]).
pub enum ClientError<E = Raw> {
    Transport(Error),
    Encode(String),
    Decode(String),
    Remote(ErrorBody<E>),
    ConnectionClosed,
}

impl ClientError<Raw> {
    /// Re-types an untyped error's body marker; used by generated clients to
    /// attach their declared error type.
    pub fn typed<E>(self) -> ClientError<E> {
        retype(self)
    }
}

impl<E> std::fmt::Debug for ClientError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Transport(e) => f.debug_tuple("Transport").field(e).finish(),

            ClientError::Encode(s) => f.debug_tuple("Encode").field(s).finish(),

            ClientError::Decode(s) => f.debug_tuple("Decode").field(s).finish(),

            ClientError::Remote(b) => f.debug_tuple("Remote").field(b).finish(),

            ClientError::ConnectionClosed => f.write_str("ConnectionClosed"),
        }
    }
}

impl<E> std::fmt::Display for ClientError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Transport(e) => write!(f, "transport: {e}"),

            ClientError::Encode(s) => write!(f, "encoding request: {s}"),

            ClientError::Decode(s) => write!(f, "decoding response: {s}"),

            ClientError::Remote(b) => write!(f, "remote error (status {:#010x})", b.code().raw()),

            ClientError::ConnectionClosed => write!(f, "connection closed before response"),
        }
    }
}

impl<E> std::error::Error for ClientError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClientError::Transport(e) => Some(e),

            _ => None,
        }
    }
}

impl<E> From<Error> for ClientError<E> {
    fn from(e: Error) -> Self {
        ClientError::Transport(e)
    }
}

/// Re-labels an untyped error onto a typed error of the same shape. The only arm
/// carrying `E` is `Remote`, whose body bytes are re-marked, not re-decoded.
fn retype<E>(err: ClientError) -> ClientError<E> {
    match err {
        ClientError::Transport(e) => ClientError::Transport(e),

        ClientError::Encode(s) => ClientError::Encode(s),

        ClientError::Decode(s) => ClientError::Decode(s),

        ClientError::Remote(b) => ClientError::Remote(b.cast()),

        ClientError::ConnectionClosed => ClientError::ConnectionClosed,
    }
}

/// Carries calls to a daemon, independent of the wire substrate. A multiplexed
/// byte stream (TCP/Unix) is one implementation; a QUIC connection mapping each
/// call to its own stream is another. The strategy and the `CallId` are hidden
/// from everything above this trait.
pub trait ClientTransport: Send + Sync + 'static {
    type Call: ClientCall;

    /// Opens a call. `streaming_input` mirrors `WireRequest`; `payload` is the
    /// opening body (empty for client/bidi opens that stream their input).
    fn open(
        &self,
        path: &str,
        streaming_input: bool,
        payload: Vec<u8>,
    ) -> impl Future<Output = Result<Self::Call, ClientError>> + Send;
}

/// One in-flight call, split into two independently-owned halves so the inbound
/// (send) and outbound (receive) directions are fully concurrent: a [`CallSink`]
/// and a [`CallSource`] can move to separate tasks and run without coordinating.
/// The substrate and the `CallId` stay hidden below this trait.
pub trait ClientCall: Send {
    type Sink: CallSink;
    type Source: CallSource;

    /// Splits the call into its send and receive halves.
    fn split(self) -> (Self::Sink, Self::Source);
}

/// The send half of a call: stream inbound items, half-close, or cancel. Owned
/// independently of its [`CallSource`], so sending never blocks on receiving.
pub trait CallSink: Send {
    fn send(&mut self, payload: Vec<u8>) -> impl Future<Output = Result<(), ClientError>> + Send;

    fn finish(&mut self) -> impl Future<Output = Result<(), ClientError>> + Send;

    fn cancel(self) -> impl Future<Output = Result<(), ClientError>> + Send;
}

/// The receive half of a call: pull demuxed outbound frames. Owned independently
/// of its [`CallSink`], so receiving never blocks on sending.
pub trait CallSource: Send {
    fn recv(&mut self) -> impl Future<Output = Result<Option<Reply>, ClientError>> + Send;
}

/// The user-facing client handle. Holds a [`ClientTransport`] and exposes the
/// transport-agnostic typed call surface the generated clients build on.
pub struct ClientConnection<T> {
    transport: T,
}

impl<T: ClientTransport> ClientConnection<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// One request, one response.
    pub async fn call<Req, Resp, E>(&self, path: &str, req: &Req) -> Result<Resp, ClientError<E>>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        let payload = postcard::to_allocvec(req).map_err(|e| ClientError::Encode(e.to_string()))?;

        let call = self
            .transport
            .open(path, false, payload)
            .await
            .map_err(retype)?;
        let (_sink, mut source) = call.split();

        decode_unary(source.recv().await.map_err(retype)?)
    }

    /// One request, a stream of responses.
    pub async fn server_stream<Req, Resp, E>(
        &self,
        path: &str,
        req: &Req,
    ) -> Result<ServerStream<T::Call, Resp, E>, ClientError<E>>
    where
        Req: Serialize,
        Resp: StreamDecode,
    {
        let payload = postcard::to_allocvec(req).map_err(|e| ClientError::Encode(e.to_string()))?;

        let call = self
            .transport
            .open(path, false, payload)
            .await
            .map_err(retype)?;
        let (_sink, source) = call.split();

        Ok(ServerStream {
            source,
            _marker: PhantomData,
        })
    }

    /// A stream of requests, one response. Mirrors the daemon: hand it the input
    /// stream, get the single response back. Items are drained and sent in order,
    /// then the call is half-closed and the response awaited.
    pub async fn client_stream<Req, Resp, E, I>(
        &self,
        path: &str,
        input: I,
    ) -> Result<Resp, ClientError<E>>
    where
        Req: StreamEncode,
        Resp: DeserializeOwned,
        I: Into<StreamArg<Req>>,
    {
        let call = self
            .transport
            .open(path, true, Vec::new())
            .await
            .map_err(retype)?;
        let (mut sink, mut source) = call.split();
        let mut input = input.into().into_inner();

        while let Some(item) = futures::StreamExt::next(&mut input).await {
            send_item(&mut sink, &item).await?;
        }

        sink.finish().await.map_err(retype)?;

        decode_unary(source.recv().await.map_err(retype)?)
    }

    /// A bidirectional stream of requests and responses, fully symmetric with the
    /// daemon: an input stream in, a response stream out, driven concurrently. The
    /// input is pumped to the wire on its own task while the returned stream yields
    /// responses, so sending and receiving are independent — cause-and-effect is up
    /// to the caller (e.g. push to a channel-backed `input`, then read responses).
    pub async fn bidi_stream<Req, Resp, E, I>(
        &self,
        path: &str,
        input: I,
    ) -> Result<BidiResponses<T::Call, Resp, E>, ClientError<E>>
    where
        Req: StreamEncode + Send + 'static,
        Resp: StreamDecode,
        I: Into<StreamArg<Req>>,
        <T::Call as ClientCall>::Sink: 'static,
    {
        let call = self
            .transport
            .open(path, true, Vec::new())
            .await
            .map_err(retype)?;
        let (mut sink, source) = call.split();
        let mut input = input.into().into_inner();

        tokio::spawn(async move {
            while let Some(item) = futures::StreamExt::next(&mut input).await {
                let Ok(bytes) = item.encode() else {
                    break;
                };

                if sink.send(bytes).await.is_err() {
                    break;
                }
            }

            let _ = sink.finish().await;
        });

        Ok(BidiResponses {
            source,
            _marker: PhantomData,
        })
    }
}

impl ClientConnection<StreamClientTransport<tokio::net::tcp::OwnedWriteHalf>> {
    /// Connects over TCP and wraps the split stream in a byte-stream transport.
    pub async fn connect_tcp(addr: impl tokio::net::ToSocketAddrs) -> Result<Self, ClientError> {
        let stream = tokio::net::TcpStream::connect(addr)
            .await
            .map_err(|e| ClientError::Transport(Error::Io(e)))?;

        let (read, write) = stream.into_split();

        Ok(Self::new(StreamClientTransport::new(read, write)))
    }
}

#[cfg(unix)]
impl ClientConnection<StreamClientTransport<tokio::net::unix::OwnedWriteHalf>> {
    /// Connects over a Unix socket and wraps the split stream in a byte-stream transport.
    pub async fn connect_unix(path: impl AsRef<std::path::Path>) -> Result<Self, ClientError> {
        let stream = tokio::net::UnixStream::connect(path)
            .await
            .map_err(|e| ClientError::Transport(Error::Io(e)))?;

        let (read, write) = stream.into_split();

        Ok(Self::new(StreamClientTransport::new(read, write)))
    }
}

/// Decodes the next response item of a server- or bidirectional-streaming call,
/// yielding `None` at end-of-stream and a terminal error as `Some(Err(..))`.
fn decode_item<Resp, E>(
    reply: Result<Option<Reply>, ClientError>,
) -> Option<Result<Resp, ClientError<E>>>
where
    Resp: StreamDecode,
{
    match reply {
        Ok(Some(Reply::Item(bytes))) => {
            Some(Resp::decode(&bytes).map_err(|e| ClientError::Decode(e.to_string())))
        }

        Ok(Some(Reply::Error { code, body })) => {
            Some(Err(ClientError::Remote(ErrorBody::new(code, body))))
        }

        Ok(Some(Reply::Response(_))) => Some(Err(ClientError::Decode(
            "unexpected unary response in stream".into(),
        ))),

        Ok(None) | Ok(Some(Reply::End)) => None,

        Err(e) => Some(Err(retype(e))),
    }
}

/// Decodes the single response of a unary or client-streaming call.
fn decode_unary<Resp, E>(reply: Option<Reply>) -> Result<Resp, ClientError<E>>
where
    Resp: DeserializeOwned,
{
    match reply {
        Some(Reply::Response(WireOutcome::Ok(bytes))) => {
            postcard::from_bytes(&bytes).map_err(|e| ClientError::Decode(e.to_string()))
        }

        Some(Reply::Response(WireOutcome::Err { code, body }))
        | Some(Reply::Error { code, body }) => Err(ClientError::Remote(ErrorBody::new(code, body))),

        None | Some(Reply::End) => Err(ClientError::ConnectionClosed),

        Some(Reply::Item(_)) => Err(ClientError::Decode(
            "unexpected stream item awaiting unary response".into(),
        )),
    }
}

/// Serializes one outbound stream item and sends it on a call's sink.
async fn send_item<S, Req, E>(sink: &mut S, item: &Req) -> Result<(), ClientError<E>>
where
    S: CallSink,
    Req: StreamEncode,
{
    let bytes = item
        .encode()
        .map_err(|e| ClientError::Encode(e.to_string()))?;

    sink.send(bytes).await.map_err(retype)
}

/// A boxed input stream a generated *trait* client accepts, so the (otherwise
/// generic) method stays object-safe. Build one from any `Stream` with `.into()`
/// (or [`new`](Self::new)); the inherent client takes an `impl Stream` directly.
///
/// It deliberately does **not** implement `Stream` itself — that would collide
/// with the blanket `From<S: Stream>` against the reflexive `From<T> for T` — so
/// the connection methods accept `impl Into<StreamArg>` and call
/// [`into_inner`](Self::into_inner) to recover the boxed stream.
pub struct StreamArg<T> {
    inner: std::pin::Pin<Box<dyn futures::Stream<Item = T> + Send>>,
}

impl<T> StreamArg<T> {
    pub fn new<S>(stream: S) -> Self
    where
        S: futures::Stream<Item = T> + Send + 'static,
    {
        Self {
            inner: Box::pin(stream),
        }
    }

    /// The boxed stream, ready to drive (it is itself a `Stream`).
    pub fn into_inner(self) -> std::pin::Pin<Box<dyn futures::Stream<Item = T> + Send>> {
        self.inner
    }
}

impl<T, S> From<S> for StreamArg<T>
where
    S: futures::Stream<Item = T> + Send + 'static,
{
    fn from(stream: S) -> Self {
        Self::new(stream)
    }
}

/// The outbound response stream of a server- or bidi-streaming call. Drive it
/// with `while let Some(item) = stream.next().await`.
pub struct ServerStream<C: ClientCall, Resp, E = Raw> {
    source: C::Source,
    _marker: Variance<(Resp, E)>,
}

impl<C, Resp, E> ServerStream<C, Resp, E>
where
    C: ClientCall,
    Resp: StreamDecode,
{
    pub async fn next(&mut self) -> Option<Result<Resp, ClientError<E>>> {
        let reply = self.source.recv().await;

        decode_item(reply)
    }
}

/// The response stream of a bidirectional call: its outbound half. Drive it with
/// `while let Some(item) = stream.next().await`, concurrently with the input
/// stream the framework is pumping on its own task.
pub struct BidiResponses<C: ClientCall, Resp, E = Raw> {
    source: C::Source,
    _marker: Variance<(Resp, E)>,
}

impl<C, Resp, E> BidiResponses<C, Resp, E>
where
    C: ClientCall,
    Resp: StreamDecode,
{
    pub async fn next(&mut self) -> Option<Result<Resp, ClientError<E>>> {
        let reply = self.source.recv().await;

        decode_item(reply)
    }
}
