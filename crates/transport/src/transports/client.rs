use std::future::Future;
use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::Error;
use crate::protocol::WireOutcome;
use crate::status::StatusCode;

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

/// One in-flight call's substrate-agnostic I/O: send inbound items, half-close,
/// receive the demuxed outbound frames, or cancel.
pub trait ClientCall: Send {
    fn send(&mut self, payload: Vec<u8>) -> impl Future<Output = Result<(), ClientError>> + Send;

    fn finish(&mut self) -> impl Future<Output = Result<(), ClientError>> + Send;

    fn recv(&mut self) -> impl Future<Output = Result<Option<Reply>, ClientError>> + Send;

    fn cancel(self) -> impl Future<Output = Result<(), ClientError>> + Send;
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

        let mut call = self.transport.open(path, false, payload).await.map_err(retype)?;

        match call.recv().await.map_err(retype)? {
            Some(Reply::Response(WireOutcome::Ok(bytes))) => {
                postcard::from_bytes(&bytes).map_err(|e| ClientError::Decode(e.to_string()))
            }

            Some(Reply::Response(WireOutcome::Err { code, body }))
            | Some(Reply::Error { code, body }) => Err(ClientError::Remote(ErrorBody::new(code, body))),

            None | Some(Reply::End) => Err(ClientError::ConnectionClosed),

            Some(Reply::Item(_)) => {
                Err(ClientError::Decode("unexpected stream item for unary call".into()))
            }
        }
    }

    /// One request, a stream of responses.
    pub async fn server_stream<Req, Resp, E>(
        &self,
        path: &str,
        req: &Req,
    ) -> Result<ServerStream<T::Call, Resp, E>, ClientError<E>>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        let payload = postcard::to_allocvec(req).map_err(|e| ClientError::Encode(e.to_string()))?;

        let call = self.transport.open(path, false, payload).await.map_err(retype)?;

        Ok(ServerStream {
            call,
            _marker: PhantomData,
        })
    }

    /// A stream of requests, one response.
    pub async fn client_stream<Req, Resp, E>(
        &self,
        path: &str,
    ) -> Result<ClientUpstream<T::Call, Req, Resp, E>, ClientError<E>>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        let call = self.transport.open(path, true, Vec::new()).await.map_err(retype)?;

        Ok(ClientUpstream {
            call,
            _marker: PhantomData,
        })
    }

    /// A bidirectional stream of requests and responses.
    pub async fn bidi_stream<Req, Resp, E>(
        &self,
        path: &str,
    ) -> Result<BidiStream<T::Call, Req, Resp, E>, ClientError<E>>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        let call = self.transport.open(path, true, Vec::new()).await.map_err(retype)?;

        Ok(BidiStream {
            call,
            _marker: PhantomData,
        })
    }
}

impl ClientConnection<StreamClientTransport<tokio::net::tcp::OwnedWriteHalf>> {
    /// Connects over TCP and wraps the split stream in a byte-stream transport.
    pub async fn connect_tcp(
        addr: impl tokio::net::ToSocketAddrs,
    ) -> Result<Self, ClientError> {
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
    pub async fn connect_unix(
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, ClientError> {
        let stream = tokio::net::UnixStream::connect(path)
            .await
            .map_err(|e| ClientError::Transport(Error::Io(e)))?;

        let (read, write) = stream.into_split();

        Ok(Self::new(StreamClientTransport::new(read, write)))
    }
}

/// Decodes the next response item of a server- or bidirectional-streaming call,
/// yielding `None` at end-of-stream and a terminal error as `Some(Err(..))`.
fn decode_item<Resp, E>(reply: Result<Option<Reply>, ClientError>) -> Option<Result<Resp, ClientError<E>>>
where
    Resp: DeserializeOwned,
{
    match reply {
        Ok(Some(Reply::Item(bytes))) => Some(
            postcard::from_bytes(&bytes).map_err(|e| ClientError::Decode(e.to_string())),
        ),

        Ok(Some(Reply::Error { code, body })) => {
            Some(Err(ClientError::Remote(ErrorBody::new(code, body))))
        }

        Ok(Some(Reply::Response(_))) => {
            Some(Err(ClientError::Decode("unexpected unary response in stream".into())))
        }

        Ok(None) | Ok(Some(Reply::End)) => None,

        Err(e) => Some(Err(retype(e))),
    }
}

/// Serializes one outbound stream item for a client/bidi call.
async fn send_item<C, Req, E>(call: &mut C, item: &Req) -> Result<(), ClientError<E>>
where
    C: ClientCall,
    Req: Serialize,
{
    let bytes = postcard::to_allocvec(item).map_err(|e| ClientError::Encode(e.to_string()))?;

    call.send(bytes).await.map_err(retype)
}

/// The outbound response stream of a server- or bidi-streaming call. Drive it
/// with `while let Some(item) = stream.next().await`.
pub struct ServerStream<C, Resp, E = Raw> {
    call: C,
    _marker: Variance<(Resp, E)>,
}

impl<C, Resp, E> ServerStream<C, Resp, E>
where
    C: ClientCall,
    Resp: DeserializeOwned,
{
    pub async fn next(&mut self) -> Option<Result<Resp, ClientError<E>>> {
        let reply = self.call.recv().await;

        decode_item(reply)
    }
}

/// The upstream half of a client-streaming call: send items, then `finish` to
/// half-close and await the single response.
pub struct ClientUpstream<C, Req, Resp, E = Raw> {
    call: C,
    _marker: Variance<(Req, Resp, E)>,
}

impl<C, Req, Resp, E> ClientUpstream<C, Req, Resp, E>
where
    C: ClientCall,
    Req: Serialize,
    Resp: DeserializeOwned,
{
    pub async fn send(&mut self, item: &Req) -> Result<(), ClientError<E>> {
        send_item(&mut self.call, item).await
    }

    pub async fn finish(mut self) -> Result<Resp, ClientError<E>> {
        self.call.finish().await.map_err(retype)?;

        match self.call.recv().await.map_err(retype)? {
            Some(Reply::Response(WireOutcome::Ok(bytes))) => {
                postcard::from_bytes(&bytes).map_err(|e| ClientError::Decode(e.to_string()))
            }

            Some(Reply::Response(WireOutcome::Err { code, body }))
            | Some(Reply::Error { code, body }) => Err(ClientError::Remote(ErrorBody::new(code, body))),

            None | Some(Reply::End) => Err(ClientError::ConnectionClosed),

            Some(Reply::Item(_)) => Err(ClientError::Decode(
                "unexpected stream item awaiting client-stream response".into(),
            )),
        }
    }
}

/// A bidirectional call: `send`/`close_send` upstream while reading responses
/// with `next`. v1 drives both halves from `&mut self`, so sends and receives
/// interleave sequentially rather than truly concurrently.
pub struct BidiStream<C, Req, Resp, E = Raw> {
    call: C,
    _marker: Variance<(Req, Resp, E)>,
}

impl<C, Req, Resp, E> BidiStream<C, Req, Resp, E>
where
    C: ClientCall,
    Req: Serialize,
    Resp: DeserializeOwned,
{
    pub async fn send(&mut self, item: &Req) -> Result<(), ClientError<E>> {
        send_item(&mut self.call, item).await
    }

    pub async fn close_send(&mut self) -> Result<(), ClientError<E>> {
        self.call.finish().await.map_err(retype)
    }

    pub async fn next(&mut self) -> Option<Result<Resp, ClientError<E>>> {
        let reply = self.call.recv().await;

        decode_item(reply)
    }
}
