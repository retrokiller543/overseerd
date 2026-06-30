//! The `reqwest` client backend.

use bytes::Bytes;
use futures::StreamExt;
use futures::stream::BoxStream;
use http::Request;
use overseerd_client::{ClientError, Unary};
use overseerd_transport::{CodecError, Decodes, Encodes};
use serde::de::DeserializeOwned;

use super::{HttpBody, HttpResponse, HttpStreaming};

/// An HTTP client transport backed by [`reqwest`].
///
/// Holds a base URL (scheme + authority, e.g. `http://localhost:3000`) that the per-call path
/// is appended to. Implements the [`Unary`] capability with a `http::Request` envelope and an
/// [`HttpResponse`] reply, so a generated controller client runs over it.
#[derive(Clone)]
pub struct ReqwestClient {
    client: reqwest::Client,
    base_url: String,
}

impl ReqwestClient {
    /// A client against `base_url` (e.g. `"http://localhost:3000"`) with a default reqwest client.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
        }
    }

    /// A client against `base_url` reusing an existing [`reqwest::Client`] (shared pool, custom
    /// timeouts/TLS, …).
    pub fn with_client(client: reqwest::Client, base_url: impl Into<String>) -> Self {
        Self {
            client,
            base_url: base_url.into(),
        }
    }
}

/// JSON is the body codec for the bytes; the body *wrapper* ([`HttpBody`]) already chose the
/// format and set the content type, so here we only forward its bytes.
impl<B: HttpBody + Send> Encodes<B> for ReqwestClient {
    fn encode(&self, value: B) -> Result<Vec<u8>, CodecError> {
        value.encode()
    }
}

/// Responses are decoded as JSON by default.
impl<R: DeserializeOwned> Decodes<R> for ReqwestClient {
    fn decode(&self, body: Vec<u8>) -> Result<R, CodecError> {
        serde_json::from_slice(&body).map_err(|e| CodecError::bad_input(e.to_string()))
    }
}

impl Unary for ReqwestClient {
    type Request<B> = Request<B>;
    type Response<R> = HttpResponse<R>;

    async fn unary<B, Resp, E>(
        &self,
        _path: &str,
        request: Request<B>,
    ) -> Result<HttpResponse<Resp>, ClientError<E>>
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

        let decoded = self
            .decode(body_bytes)
            .map_err(|e| ClientError::Decode(e.to_string()))?;

        Ok(HttpResponse::new(status, headers, decoded))
    }
}

impl HttpStreaming for ReqwestClient {
    type ByteStream = BoxStream<'static, Result<Bytes, ClientError>>;

    async fn open_stream<B>(&self, request: Request<B>) -> Result<Self::ByteStream, ClientError>
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

/// Maps a reqwest network failure onto the transport arm of [`ClientError`].
fn net_err<E>(error: reqwest::Error) -> ClientError<E> {
    ClientError::Transport(overseerd_transport::Error::Io(std::io::Error::other(
        error.to_string(),
    )))
}
