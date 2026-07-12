//! The `hyper` client backend.

use bytes::Bytes;
use futures::Stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use http::{Request, Uri};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, BodyStream, Full, StreamBody};
use hyper::body::Frame;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use overseerd_client::{ClientError, MaybeSend, Transport, Unary};
use overseerd_transport::{CodecError, Decodes, Encodes, Error as TransportError};
use serde::de::DeserializeOwned;

use super::{ClientInterceptor, HttpBody, HttpClientStreaming, HttpResponse, HttpStreaming};
#[cfg(all(feature = "ws", feature = "client"))]
use super::{WebsocketClient, WsStatus};

type HyperBody = UnsyncBoxBody<Bytes, TransportError>;

/// An HTTP client transport backed by `hyper` (via `hyper-util`'s pooled client).
///
/// Plain HTTP only — no TLS feature is enabled, so `openssl`/`rsa` stay out of the tree;
/// terminate TLS at a reverse proxy, or use the [`ReqwestClient`](super::ReqwestClient) backend.
/// Holds a base URL (scheme + authority) the per-call path is appended to, and implements the
/// [`Unary`] capability with a `http::Request` envelope and an [`HttpResponse`] reply.
#[derive(Clone)]
pub struct HyperClient<W = (), I: ClientInterceptor = ()> {
    client: Client<HttpConnector, HyperBody>,
    base_url: String,
    interceptor: I,
    websocket: W,
}

impl HyperClient<(), ()> {
    /// A client against `base_url` (e.g. `"http://localhost:3000"`) with a default pooled client.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::builder(TokioExecutor::new()).build_http(),
            base_url: base_url.into(),
            interceptor: (),
            websocket: (),
        }
    }
}

impl<W, I: ClientInterceptor> HyperClient<W, I> {
    /// Attaches a websocket request/reply transport while keeping this HTTP backend.
    pub fn with_websocket<Ws>(self, websocket: Ws) -> HyperClient<Ws, I> {
        HyperClient {
            client: self.client,
            base_url: self.base_url,
            interceptor: self.interceptor,
            websocket,
        }
    }

    /// Replaces the interceptor type stored directly on this client.
    pub fn with_interceptor<J: ClientInterceptor>(self, interceptor: J) -> HyperClient<W, J> {
        HyperClient {
            client: self.client,
            base_url: self.base_url,
            interceptor,
            websocket: self.websocket,
        }
    }

    /// The interceptor stored on this transport.
    pub fn interceptor(&self) -> &I {
        &self.interceptor
    }

    fn prepare_parts<E>(
        &self,
        mut parts: http::request::Parts,
    ) -> Result<http::request::Parts, ClientError<http::StatusCode, E>>
    where
        I: ClientInterceptor,
    {
        parts.uri = format!("{}{}", self.base_url, parts.uri)
            .parse::<Uri>()
            .map_err(|error| self.fail(ClientError::Encode(error.to_string())))?;
        self.interceptor.on_request(&mut parts);
        Ok(parts)
    }

    fn fail<E>(&self, error: ClientError<http::StatusCode, E>) -> ClientError<http::StatusCode, E>
    where
        I: ClientInterceptor,
    {
        self.interceptor.on_error(&error);
        error
    }

    /// Encodes the body and resolves the path-only URI against the base authority, producing the
    /// concrete `hyper` request. Shared by the unary and streaming calls.
    fn build_request<B, E>(
        &self,
        request: Request<B>,
    ) -> Result<Request<HyperBody>, ClientError<http::StatusCode, E>>
    where
        Self: Encodes<B>,
        I: ClientInterceptor,
    {
        let (parts, body) = request.into_parts();
        let bytes = self
            .encode(body)
            .map_err(|error| self.fail(ClientError::Encode(error.to_string())))?;
        let parts = self.prepare_parts(parts)?;

        let mut builder = Request::builder().method(parts.method).uri(parts.uri);

        if let Some(headers) = builder.headers_mut() {
            *headers = parts.headers;
        }

        builder
            .body(full_body(Bytes::from(bytes)))
            .map_err(|error| self.fail(ClientError::Encode(error.to_string())))
    }

    fn build_stream_request<S, E>(
        &self,
        request: Request<S>,
    ) -> Result<Request<HyperBody>, ClientError<http::StatusCode, E>>
    where
        S: Stream<Item = Result<Bytes, CodecError>> + Send + 'static,
        I: ClientInterceptor,
    {
        let (parts, body) = request.into_parts();
        let parts = self.prepare_parts(parts)?;

        let mut builder = Request::builder().method(parts.method).uri(parts.uri);

        if let Some(headers) = builder.headers_mut() {
            *headers = parts.headers;
        }

        builder
            .body(stream_body(body))
            .map_err(|error| self.fail(ClientError::Encode(error.to_string())))
    }
}

/// The body wrapper ([`HttpBody`]) already chose the format and content type; forward its bytes.
impl<W, I, B> Encodes<B> for HyperClient<W, I>
where
    W: Send + Sync,
    I: ClientInterceptor + Send + Sync,
    B: HttpBody + Send,
{
    fn encode(&self, value: B) -> Result<Vec<u8>, CodecError> {
        value.encode()
    }
}

/// Responses are decoded as JSON by default.
impl<W, I, R> Decodes<R> for HyperClient<W, I>
where
    W: Send + Sync,
    I: ClientInterceptor + Send + Sync,
    R: DeserializeOwned,
{
    fn decode(&self, body: Vec<u8>) -> Result<R, CodecError> {
        serde_json::from_slice(&body).map_err(|e| CodecError::bad_input(e.to_string()))
    }
}

/// The HTTP client's protocol status is the genuine [`http::StatusCode`].
impl<W, I> Transport for HyperClient<W, I>
where
    W: Send + Sync,
    I: ClientInterceptor + Send + Sync,
{
    type Status = http::StatusCode;
}

impl<W, I> Unary for HyperClient<W, I>
where
    W: Send + Sync,
    I: ClientInterceptor + Send + Sync,
{
    type Request<B> = Request<B>;
    type Response<R> = HttpResponse<R>;

    async fn unary<B, Resp, E>(
        &self,
        _path: &str,
        request: Request<B>,
    ) -> Result<HttpResponse<Resp>, ClientError<http::StatusCode, E>>
    where
        Self: Encodes<B> + Decodes<Resp>,
        B: MaybeSend,
        Resp: MaybeSend,
    {
        let request = self.build_request(request)?;
        let response = self
            .client
            .request(request)
            .await
            .map_err(|error| self.fail(net_err(error)))?;
        let (mut parts, body) = response.into_parts();
        self.interceptor.on_response(&mut parts);

        let body_bytes = body
            .collect()
            .await
            .map_err(|error| self.fail(net_err(error)))?
            .to_bytes()
            .to_vec();

        if !parts.status.is_success() {
            return Err(self.fail(super::remote_error(parts.status, body_bytes).typed()));
        }

        let decoded = self
            .decode(body_bytes)
            .map_err(|error| self.fail(ClientError::Decode(error.to_string())))?;

        Ok(HttpResponse::new(parts.status, parts.headers, decoded))
    }
}

impl<W, I> HttpStreaming for HyperClient<W, I>
where
    W: Send + Sync,
    I: ClientInterceptor + Send + Sync,
{
    type ByteStream = BoxStream<'static, Result<Bytes, ClientError<http::StatusCode>>>;

    async fn open_stream<B>(
        &self,
        request: Request<B>,
    ) -> Result<Self::ByteStream, ClientError<http::StatusCode>>
    where
        Self: Encodes<B>,
        B: Send,
    {
        let request = self.build_request(request)?;
        let response = self
            .client
            .request(request)
            .await
            .map_err(|error| self.fail(net_err(error)))?;
        let (mut parts, body) = response.into_parts();
        self.interceptor.on_response(&mut parts);

        // A non-success status is a pre-stream failure; surface it as the outer `Err` rather than
        // streaming an error body as items.
        if !parts.status.is_success() {
            let body = body
                .collect()
                .await
                .map_err(|error| self.fail(net_err(error)))?
                .to_bytes()
                .to_vec();

            return Err(self.fail(super::remote_error(parts.status, body)));
        }

        // Body frames → data-chunk stream; trailer frames are dropped.
        let stream = BodyStream::new(body)
            .filter_map(|frame| async move {
                match frame {
                    Ok(frame) => frame.into_data().ok().map(Ok),
                    Err(error) => Some(Err(net_err(error))),
                }
            })
            .boxed();

        Ok(stream)
    }
}

impl<W, I> HttpClientStreaming for HyperClient<W, I>
where
    W: Send + Sync,
    I: ClientInterceptor + Send + Sync,
{
    async fn send_stream<S, Resp, E>(
        &self,
        request: Request<S>,
    ) -> Result<HttpResponse<Resp>, ClientError<http::StatusCode, E>>
    where
        Self: Decodes<Resp>,
        S: Stream<Item = Result<Bytes, CodecError>> + Send + 'static,
        Resp: Send,
    {
        let request = self.build_stream_request(request)?;
        let response = self
            .client
            .request(request)
            .await
            .map_err(|error| self.fail(net_err(error)))?;
        let (mut parts, body) = response.into_parts();
        self.interceptor.on_response(&mut parts);

        let body_bytes = body
            .collect()
            .await
            .map_err(|error| self.fail(net_err(error)))?
            .to_bytes()
            .to_vec();

        if !parts.status.is_success() {
            return Err(self.fail(super::remote_error(parts.status, body_bytes).typed()));
        }

        let decoded = self
            .decode(body_bytes)
            .map_err(|error| self.fail(ClientError::Decode(error.to_string())))?;

        Ok(HttpResponse::new(parts.status, parts.headers, decoded))
    }
}

#[cfg(all(feature = "ws", feature = "client"))]
impl<W, I, P, Req, Resp> WebsocketClient<P, Req, Resp> for HyperClient<W, I>
where
    W: WebsocketClient<P, Req, Resp>,
    I: ClientInterceptor + Send + Sync,
    P: super::WebsocketClientProtocol,
    Req: Send,
    Resp: Send,
{
    async fn websocket_call(
        &self,
        destination: &'static str,
        payload: Req,
    ) -> Result<Resp, ClientError<WsStatus>>
    where
        Req: Send,
        Resp: Send,
    {
        self.websocket.websocket_call(destination, payload).await
    }
}

fn full_body(bytes: Bytes) -> HyperBody {
    Full::new(bytes)
        .map_err(|never| match never {})
        .boxed_unsync()
}

fn stream_body<S>(stream: S) -> HyperBody
where
    S: Stream<Item = Result<Bytes, CodecError>> + Send + 'static,
{
    let stream = stream.map(|chunk| {
        chunk
            .map(Frame::data)
            .map_err(|e| TransportError::Serialization(e.to_string()))
    });

    BodyExt::boxed_unsync(StreamBody::new(stream))
}

/// Maps a hyper network failure onto the transport arm of [`ClientError`] (status `S` and error
/// body `E` are inferred from the call site; the transport arm carries neither).
fn net_err<T, S, E>(error: T) -> ClientError<S, E>
where
    T: std::fmt::Display,
{
    ClientError::Transport(overseerd_transport::Error::Io(std::io::Error::other(
        error.to_string(),
    )))
}
