//! The `reqwest` client backend.

use bytes::Bytes;
use futures::Stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use http::Request;
use overseerd_client::{ClientError, Transport, Unary};
use overseerd_transport::{CodecError, Decodes, Encodes};
use serde::de::DeserializeOwned;

use super::{HttpBody, HttpClientStreaming, HttpResponse, HttpStreaming};
#[cfg(all(feature = "ws", feature = "client"))]
use super::{WebsocketClient, WsStatus};

/// An HTTP client transport backed by [`reqwest`].
///
/// Holds a base URL (scheme + authority, e.g. `http://localhost:3000`) that the per-call path
/// is appended to. Implements the [`Unary`] capability with a `http::Request` envelope and an
/// [`HttpResponse`] reply, so a generated controller client runs over it.
#[derive(Clone)]
pub struct ReqwestClient<W = ()> {
    client: reqwest::Client,
    base_url: String,
    websocket: W,
}

impl ReqwestClient<()> {
    /// A client against `base_url` (e.g. `"http://localhost:3000"`) with a default reqwest client.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            websocket: (),
        }
    }

    /// A client against `base_url` reusing an existing [`reqwest::Client`] (shared pool, custom
    /// timeouts/TLS, …).
    pub fn with_client(client: reqwest::Client, base_url: impl Into<String>) -> Self {
        Self {
            client,
            base_url: base_url.into(),
            websocket: (),
        }
    }
}

impl<W> ReqwestClient<W> {
    /// Attaches a websocket request/reply transport while keeping this HTTP backend.
    pub fn with_websocket<Ws>(self, websocket: Ws) -> ReqwestClient<Ws> {
        ReqwestClient {
            client: self.client,
            base_url: self.base_url,
            websocket,
        }
    }
}

/// JSON is the body codec for the bytes; the body *wrapper* ([`HttpBody`]) already chose the
/// format and set the content type, so here we only forward its bytes.
impl<W, B> Encodes<B> for ReqwestClient<W>
where
    W: Send + Sync,
    B: HttpBody + Send,
{
    fn encode(&self, value: B) -> Result<Vec<u8>, CodecError> {
        value.encode()
    }
}

/// Responses are decoded as JSON by default.
impl<W, R> Decodes<R> for ReqwestClient<W>
where
    W: Send + Sync,
    R: DeserializeOwned,
{
    fn decode(&self, body: Vec<u8>) -> Result<R, CodecError> {
        serde_json::from_slice(&body).map_err(|e| CodecError::bad_input(e.to_string()))
    }
}

/// The HTTP client's protocol status is the genuine [`http::StatusCode`].
impl<W> Transport for ReqwestClient<W>
where
    W: Send + Sync,
{
    type Status = http::StatusCode;
}

impl<W> Unary for ReqwestClient<W>
where
    W: Send + Sync,
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
        B: Send,
        Resp: Send,
    {
        let (parts, body) = request.into_parts();
        let bytes = self
            .encode(body)
            .map_err(|e| ClientError::Encode(e.to_string()))?;

        let url = format!("{}{}", self.base_url, parts.uri);

        let response = self
            .client
            .request(parts.method, url)
            .headers(parts.headers)
            .body(bytes)
            .send()
            .await
            .map_err(net_err)?;

        let status = response.status();
        let headers = response.headers().clone();
        let body_bytes = response.bytes().await.map_err(net_err)?.to_vec();

        if !status.is_success() {
            return Err(super::remote_error(status, body_bytes).typed());
        }

        let decoded = self
            .decode(body_bytes)
            .map_err(|e| ClientError::Decode(e.to_string()))?;

        Ok(HttpResponse::new(status, headers, decoded))
    }
}

impl<W> HttpStreaming for ReqwestClient<W>
where
    W: Send + Sync,
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
        let (parts, body) = request.into_parts();
        let bytes = self
            .encode(body)
            .map_err(|e| ClientError::Encode(e.to_string()))?;

        let url = format!("{}{}", self.base_url, parts.uri);

        let response = self
            .client
            .request(parts.method, url)
            .headers(parts.headers)
            .body(bytes)
            .send()
            .await
            .map_err(net_err)?;

        // A non-success status is a pre-stream failure (the handler errored before streaming);
        // surface it as the outer `Err` rather than streaming an error body as items.
        if !response.status().is_success() {
            let status = response.status();
            let body = response.bytes().await.map_err(net_err)?.to_vec();

            return Err(super::remote_error(status, body));
        }

        Ok(response
            .bytes_stream()
            .map(|chunk| chunk.map_err(net_err))
            .boxed())
    }
}

impl<W> HttpClientStreaming for ReqwestClient<W>
where
    W: Send + Sync,
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
        let (parts, body) = request.into_parts();
        let url = format!("{}{}", self.base_url, parts.uri);

        let response = self
            .client
            .request(parts.method, url)
            .headers(parts.headers)
            .body(reqwest::Body::wrap_stream(body))
            .send()
            .await
            .map_err(net_err)?;

        // A client-streaming call returns a unary response. Preserve non-success statuses as
        // remote errors before decoding the success body.
        let status = response.status();
        let headers = response.headers().clone();
        let body_bytes = response.bytes().await.map_err(net_err)?.to_vec();

        if !status.is_success() {
            return Err(super::remote_error(status, body_bytes).typed());
        }

        let decoded = self
            .decode(body_bytes)
            .map_err(|e| ClientError::Decode(e.to_string()))?;

        Ok(HttpResponse::new(status, headers, decoded))
    }
}

#[cfg(all(feature = "ws", feature = "client"))]
impl<W, P, Req, Resp> WebsocketClient<P, Req, Resp> for ReqwestClient<W>
where
    W: WebsocketClient<P, Req, Resp>,
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

/// Maps a reqwest network failure onto the transport arm of [`ClientError`] (status `S` and error
/// body `E` are inferred from the call site; the transport arm carries neither).
fn net_err<S, E>(error: reqwest::Error) -> ClientError<S, E> {
    ClientError::Transport(overseerd_transport::Error::Io(std::io::Error::other(
        error.to_string(),
    )))
}
