//! The `reqwest` client backend.

#[cfg(not(target_family = "wasm"))]
use bytes::Bytes;
#[cfg(not(target_family = "wasm"))]
use futures::Stream;
#[cfg(not(target_family = "wasm"))]
use futures::StreamExt;
#[cfg(not(target_family = "wasm"))]
use futures::stream::BoxStream;
use std::sync::{Arc, RwLock};

use http::Request;
use http::header::HeaderMap;
use overseerd_client::{ClientError, MaybeSend, Transport, Unary};
use overseerd_transport::{CodecError, Decodes, Encodes};
use serde::de::DeserializeOwned;

use super::{HttpBody, HttpResponse};
#[cfg(not(target_family = "wasm"))]
use super::{HttpClientStreaming, HttpStreaming};
#[cfg(all(feature = "ws", feature = "client", not(target_family = "wasm")))]
use super::{WebsocketClient, WsStatus};

/// A callback producing the default headers to attach to every request — the transport-level hook for
/// dynamic auth: it runs per request, so returning `authorization` from a token store applies (and
/// refreshes) the token everywhere without rebuilding clients. Per-request and per-call headers still
/// win over what it returns. Set with [`ReqwestClient::set_header_provider`] (native): the client
/// transport must be `Send + Sync` (the codec traits require it), so the provider is a Rust closure —
/// a wasm/browser client instead passes per-call headers to the generated `{method}(…, headers?)`.
pub type HeaderProvider = Arc<dyn Fn() -> HeaderMap + Send + Sync>;

// A shared, settable slot for the provider, so setting it on one handle is seen by every client
// cloned from the same transport. Empty on wasm (no `set_header_provider` there) but kept uniform so
// the transport type — and its `Send + Sync` — is identical across targets.
type ProviderSlot = Arc<RwLock<Option<HeaderProvider>>>;

/// An HTTP client transport backed by [`reqwest`].
///
/// Holds a base URL (scheme + authority, e.g. `http://localhost:3000`) that the per-call path
/// is appended to. Implements the [`Unary`] capability with a `http::Request` envelope and an
/// [`HttpResponse`] reply, so a generated controller client runs over it.
#[derive(Clone)]
pub struct ReqwestClient<W = ()> {
    client: reqwest::Client,
    base_url: String,
    /// The default-header provider (see [`HeaderProvider`]), shared across clones of the transport so
    /// it can be installed on the `Connection` and picked up by every client built from it.
    header_provider: ProviderSlot,
    // Read only by the `WebsocketClient` delegate, which is native-only; on wasm the field is
    // carried (so `with_websocket`/`W` stay uniform across targets) but never read.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    websocket: W,
}

impl ReqwestClient<()> {
    /// A client against `base_url` (e.g. `"http://localhost:3000"`) with a default reqwest client.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            header_provider: ProviderSlot::default(),
            websocket: (),
        }
    }

    /// A client against `base_url` reusing an existing [`reqwest::Client`] (shared pool, custom
    /// timeouts/TLS, …).
    pub fn with_client(client: reqwest::Client, base_url: impl Into<String>) -> Self {
        Self {
            client,
            base_url: base_url.into(),
            header_provider: ProviderSlot::default(),
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
            header_provider: self.header_provider,
            websocket,
        }
    }

    /// Installs the [`HeaderProvider`] callback, replacing any previous one. It runs on every request
    /// (through this client and every client sharing the transport), so returning a fresh
    /// `authorization` header is the auth hook. A request's own headers (content type, per-call
    /// headers) still take precedence. Native-only — a wasm client passes per-call headers instead.
    #[cfg(not(target_family = "wasm"))]
    pub fn set_header_provider<F>(&self, provider: F)
    where
        F: Fn() -> HeaderMap + Send + Sync + 'static,
    {
        *self
            .header_provider
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(Arc::new(provider));
    }

    /// Merges the provider's default headers (if one is installed) under a request's own headers (the
    /// request's — content type and any per-call headers the generated method already folded in — win
    /// on a clash).
    fn merged_headers(&self, request_headers: &HeaderMap) -> HeaderMap {
        let mut merged = match &*self
            .header_provider
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
        {
            Some(provider) => provider(),
            None => HeaderMap::new(),
        };

        for (name, value) in request_headers {
            merged.insert(name.clone(), value.clone());
        }

        merged
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
        B: MaybeSend,
        Resp: MaybeSend,
    {
        let (parts, body) = request.into_parts();
        let bytes = self
            .encode(body)
            .map_err(|e| ClientError::Encode(e.to_string()))?;

        let url = format!("{}{}", self.base_url, parts.uri);

        let response = self
            .client
            .request(parts.method, url)
            .headers(self.merged_headers(&parts.headers))
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

#[cfg(not(target_family = "wasm"))]
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
            .headers(self.merged_headers(&parts.headers))
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

#[cfg(not(target_family = "wasm"))]
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
            .headers(self.merged_headers(&parts.headers))
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

#[cfg(all(feature = "ws", feature = "client", not(target_family = "wasm")))]
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
