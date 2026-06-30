//! The Overseerd protocol-agnostic client *contract*.
//!
//! The framework implements no client calls. It defines **capabilities** a protocol may
//! support — one trait each — and a protocol (`overseerd-rpc`, a future HTTP binding, …)
//! implements the subset it can. A protocol *declares* support by implementing a capability
//! and *refuses* by simply not: a client-streaming call over HTTP/1.1 is then a compile
//! error — a protocol limitation expressed in the type system, never a framework limit.
//!
//! | Capability | trait | HTTP/1.1 | HTTP/2 | gRPC | custom RPC |
//! |---|---|:-:|:-:|:-:|:-:|
//! | one req → one resp | [`Unary`] | ✓ | ✓ | ✓ | ✓ |
//! | one req → many resp | [`ServerStreaming`] | ✓ | ✓ | ✓ | ✓ |
//! | many req → one resp | [`ClientStreaming`] | — | ✓ | ✓ | ✓ |
//! | many req ↔ many resp | [`BidiStreaming`] | — | ✓ | ✓ | ✓ |
//!
//! The protocol owns **everything** behind each method: body encoding and parsing, framing,
//! routing, and streaming. A streaming capability returns the protocol's own response
//! [`Stream`] as an associated type — a real contract that the result *is* a stream of
//! decoded items, not an opaque box that would pretend any stream fits.
//!
//! A generated client (`FooClient<C>`) bounds `C` on exactly the capabilities its RPCs need
//! and delegates to them, so the same generated code runs over any protocol providing those
//! capabilities.
//!
//! The framework assumes **no serialization**: it never bounds a message on `serde` (a
//! protocol might use rkyv, protobuf, …). Instead a protocol declares what it can carry by
//! implementing [`Encodes<T>`] / [`Decodes<T>`] for those message types (typically a blanket
//! `impl<T: Serialize> Encodes<T>`, or `impl<T: Archive> Encodes<T>`, …). A capability method
//! is bound `where Self: Encodes<Req> + Decodes<Resp>`, so a message the protocol cannot
//! serialize is a compile error — protocol-defined, never framework-assumed.

use std::future::Future;
use std::marker::PhantomData;

use futures::Stream;

use overseerd_transport::Error;

pub use overseerd_transport::{CodecError, Decodes, Encodes};

// ---------------------------------------------------------------------------
// Shared vocabulary: error currency and the streaming-input wrapper. These are
// protocol-neutral, so they live in the contract crate.
// ---------------------------------------------------------------------------

/// Marker for an error body whose payload type is not known statically (the generic call
/// path). Typed clients substitute the method's declared error type.
#[derive(Debug, Clone, Copy)]
pub struct Raw;

/// `Send`/`Sync`-preserving phantom marker that doesn't bind its type to the owning struct
/// (the markers are only carried for type inference).
type Variance<T> = PhantomData<fn() -> T>;

/// The client mirror of the server's error response: a protocol-defined status `S` plus the raw
/// error body bytes. The status is opaque to the framework (the protocol picks `S`; RPC uses its
/// packed `transport::StatusCode`, HTTP uses `http::StatusCode`) — only the caller interprets it.
/// The body is deserialized into `T` lazily and best-effort, so a body the handler serialized as a
/// different type (or as raw, non-postcard bytes) degrades to a failed `deserialize` while
/// `code`/`raw` stay usable.
pub struct ErrorBody<S, T = Raw> {
    code: S,
    body: Vec<u8>,
    _marker: Variance<T>,
}

impl<S, T> ErrorBody<S, T> {
    /// Wraps a status and raw body bytes. Public so protocol impls can build it.
    pub fn new(code: S, body: Vec<u8>) -> Self {
        Self {
            code,
            body,
            _marker: PhantomData,
        }
    }

    /// The protocol-defined status. Opaque to the framework; the caller interprets it.
    pub fn code(&self) -> S
    where
        S: Copy,
    {
        self.code
    }

    pub fn raw(&self) -> &[u8] {
        &self.body
    }

    pub fn into_raw(self) -> Vec<u8> {
        self.body
    }

    /// Re-types the body marker without touching the bytes (or the status), e.g. to attach a known
    /// body type to an otherwise [`Raw`] error.
    pub fn cast<U>(self) -> ErrorBody<S, U> {
        ErrorBody::new(self.code, self.body)
    }
}

impl<S: Clone, T> Clone for ErrorBody<S, T> {
    fn clone(&self) -> Self {
        Self::new(self.code.clone(), self.body.clone())
    }
}

impl<S: std::fmt::Debug, T> std::fmt::Debug for ErrorBody<S, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ErrorBody")
            .field("code", &self.code)
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// Everything that can go wrong on a client call. Generic over the error body type `E` so
/// typed clients surface an [`ErrorBody<E>`] with a ready [`deserialize`](ErrorBody::deserialize);
/// the generic path leaves it [`Raw`].
///
/// Debug/Display/Error are implemented by hand so no bound is placed on `E` (it lives only as
/// a phantom marker inside [`ErrorBody`]).
pub enum ClientError<S, E = Raw> {
    Transport(Error),
    Encode(String),
    Decode(String),
    Remote(ErrorBody<S, E>),
    ConnectionClosed,
}

impl<S> ClientError<S, Raw> {
    /// Re-types an untyped error's body marker (keeping its status `S`); used by generated clients
    /// to attach their declared error type.
    pub fn typed<E>(self) -> ClientError<S, E> {
        retype(self)
    }
}

impl<S: std::fmt::Debug, E> std::fmt::Debug for ClientError<S, E> {
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

impl<S: std::fmt::Debug, E> std::fmt::Display for ClientError<S, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Transport(e) => write!(f, "transport: {e}"),

            ClientError::Encode(s) => write!(f, "encoding request: {s}"),

            ClientError::Decode(s) => write!(f, "decoding response: {s}"),

            ClientError::Remote(b) => write!(f, "remote error (status {:?})", b.code),

            ClientError::ConnectionClosed => write!(f, "connection closed before response"),
        }
    }
}

impl<S: std::fmt::Debug, E> std::error::Error for ClientError<S, E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClientError::Transport(e) => Some(e),

            _ => None,
        }
    }
}

impl<S, E> From<Error> for ClientError<S, E> {
    fn from(e: Error) -> Self {
        ClientError::Transport(e)
    }
}

/// Re-labels an untyped error onto a typed error of the same shape, preserving its status `S`. The
/// only arm carrying `E` is `Remote`, whose body bytes are re-marked, not re-decoded. Used by
/// protocol impls and generated clients.
pub fn retype<S, E>(err: ClientError<S, Raw>) -> ClientError<S, E> {
    match err {
        ClientError::Transport(e) => ClientError::Transport(e),

        ClientError::Encode(s) => ClientError::Encode(s),

        ClientError::Decode(s) => ClientError::Decode(s),

        ClientError::Remote(b) => ClientError::Remote(b.cast()),

        ClientError::ConnectionClosed => ClientError::ConnectionClosed,
    }
}

/// A boxed input stream a streaming-request capability accepts. Build one from any `Stream`
/// with `.into()` (or [`new`](Self::new)).
///
/// It deliberately does **not** implement `Stream` itself — that would collide with the
/// blanket `From<S: Stream>` against the reflexive `From<T> for T` — so capability methods
/// accept `impl Into<StreamArg>` and call [`into_inner`](Self::into_inner) to recover it.
pub struct StreamArg<T> {
    inner: std::pin::Pin<Box<dyn Stream<Item = T> + Send>>,
}

impl<T> StreamArg<T> {
    pub fn new<S>(stream: S) -> Self
    where
        S: Stream<Item = T> + Send + 'static,
    {
        Self {
            inner: Box::pin(stream),
        }
    }

    /// The boxed stream, ready to drive (it is itself a `Stream`).
    pub fn into_inner(self) -> std::pin::Pin<Box<dyn Stream<Item = T> + Send>> {
        self.inner
    }
}

impl<T, S> From<S> for StreamArg<T>
where
    S: Stream<Item = T> + Send + 'static,
{
    fn from(stream: S) -> Self {
        Self::new(stream)
    }
}

// ---------------------------------------------------------------------------
// The capability contract. A protocol implements the traits it supports; the
// serialization bounds are expressed through `Encodes`/`Decodes` on `Self`, so the
// message types are whatever the protocol can carry — never a fixed serde bound.
// ---------------------------------------------------------------------------

/// The base every capability shares: a transport carries a protocol-defined [`Status`](Self::Status)
/// on its errors. The framework never interprets it — it is set by the handler author and read by
/// the client caller — so it is a bare associated type with no bounds. RPC sets it to its packed
/// `transport::StatusCode`; HTTP sets it to `http::StatusCode`. Every capability requires this, so a
/// transport declares its status once and it threads through every call shape's errors.
pub trait Transport: Send + Sync {
    /// The protocol-defined status carried on this transport's error responses. Opaque to the
    /// framework.
    type Status;
}

/// One request, one response. Every protocol supports this. The implementation owns body
/// encoding (via [`Encodes`]) and response parsing (via [`Decodes`]).
pub trait Unary: Transport {
    /// The request envelope this transport accepts, over a body type `B`.
    ///
    /// This is the seam that lets one generated client shape span very different wire
    /// protocols. A protocol that needs only a body passes it straight through
    /// (`type Request<B> = B`); a protocol that needs more — HTTP's method, headers, and
    /// path — wraps the body in its own envelope (`type Request<B> = HttpRequest<B>`), and
    /// its `unary` impl reads those fields off the *concrete* envelope (a method generic
    /// `Req` could not be destructured). A generated client pins the envelope per call with a
    /// `Unary<Request<B> = ..>` bound, so it composes the exact request its transport expects.
    type Request<B>;

    /// The response envelope this transport returns, over a decoded body `R`. The dual of
    /// [`Request`](Self::Request): RPC returns the body straight (`type Response<R> = R`); HTTP
    /// returns an `HttpResponse<R>` that carries the status and headers yet `Deref`s/`AsRef`s
    /// into `R`, so the body stays one `.` away while status/headers remain reachable. HTTP
    /// implementations may map non-success statuses into [`ClientError::Remote`] before decoding
    /// the success body. A generated client pins it with a `Response<R> = ..` bound.
    type Response<R>;

    fn unary<B, Resp, E>(
        &self,
        path: &str,
        request: Self::Request<B>,
    ) -> impl Future<Output = Result<Self::Response<Resp>, ClientError<Self::Status, E>>> + Send
    where
        Self: Encodes<B> + Decodes<Resp>,
        B: Send,
        Resp: Send;
}

/// One request, a stream of responses (HTTP/1.1 chunked/SSE, HTTP/2, gRPC, custom RPC). The
/// response stream is the protocol's own type — a contract that the result *is* a [`Stream`]
/// of decoded items, not a box.
pub trait ServerStreaming: Transport {
    /// The response stream this protocol yields for a server-streaming call. It exists only
    /// when the protocol can decode `Resp` — the decoding is the protocol's, in the stream.
    type Responses<Resp, E>: Stream<Item = Result<Resp, ClientError<Self::Status, E>>> + Send
    where
        Self: Decodes<Resp>;

    fn server_stream<Req, Resp, E>(
        &self,
        path: &str,
        request: Req,
    ) -> impl Future<Output = Result<Self::Responses<Resp, E>, ClientError<Self::Status, E>>> + Send
    where
        Self: Encodes<Req> + Decodes<Resp>,
        Req: Send,
        Resp: Send;
}

/// A stream of requests, one response (HTTP/2, gRPC, custom RPC — *not* HTTP/1.1, which has
/// no streamed request body, so it does not implement this trait).
pub trait ClientStreaming: Transport {
    fn client_stream<Req, Resp, E, I>(
        &self,
        path: &str,
        requests: I,
    ) -> impl Future<Output = Result<Resp, ClientError<Self::Status, E>>> + Send
    where
        Self: Encodes<Req> + Decodes<Resp>,
        Req: Send + 'static,
        Resp: Send,
        I: Into<StreamArg<Req>> + Send;
}

/// A bidirectional stream of requests and responses (HTTP/2, gRPC, custom RPC — *not*
/// HTTP/1.1). The request stream is pumped concurrently with reading the response stream.
pub trait BidiStreaming: Transport {
    /// The response stream this protocol yields for a bidirectional call. It exists only when
    /// the protocol can decode `Resp`.
    type Responses<Resp, E>: Stream<Item = Result<Resp, ClientError<Self::Status, E>>> + Send
    where
        Self: Decodes<Resp>;

    fn bidi_stream<Req, Resp, E, I>(
        &self,
        path: &str,
        requests: I,
    ) -> impl Future<Output = Result<Self::Responses<Resp, E>, ClientError<Self::Status, E>>> + Send
    where
        Self: Encodes<Req> + Decodes<Resp>,
        Req: Send + 'static,
        Resp: Send,
        I: Into<StreamArg<Req>> + Send;
}
