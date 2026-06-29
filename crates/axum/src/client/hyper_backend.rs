//! The `hyper` client backend.

use bytes::Bytes;
use http::{Request, Uri};
use http_body_util::{BodyExt, Full};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use overseerd_client::{ClientError, Unary};
use overseerd_transport::{CodecError, Decodes, Encodes};
use serde::de::DeserializeOwned;

use super::{HttpBody, HttpResponse};

/// An HTTP client transport backed by `hyper` (via `hyper-util`'s pooled client).
///
/// Plain HTTP only — no TLS feature is enabled, so `openssl`/`rsa` stay out of the tree;
/// terminate TLS at a reverse proxy, or use the [`ReqwestClient`](super::ReqwestClient) backend.
/// Holds a base URL (scheme + authority) the per-call path is appended to, and implements the
/// [`Unary`] capability with a `http::Request` envelope and an [`HttpResponse`] reply.
#[derive(Clone)]
pub struct HyperClient {
    client: Client<HttpConnector, Full<Bytes>>,
    base_url: String,
}

impl HyperClient {
    /// A client against `base_url` (e.g. `"http://localhost:3000"`) with a default pooled client.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::builder(TokioExecutor::new()).build_http(),
            base_url: base_url.into(),
        }
    }
}

/// The body wrapper ([`HttpBody`]) already chose the format and content type; forward its bytes.
impl<B: HttpBody + Send> Encodes<B> for HyperClient {
    fn encode(&self, value: B) -> Result<Vec<u8>, CodecError> {
        value.encode()
    }
}

/// Responses are decoded as JSON by default.
impl<R: DeserializeOwned> Decodes<R> for HyperClient {
    fn decode(&self, body: Vec<u8>) -> Result<R, CodecError> {
        serde_json::from_slice(&body).map_err(|e| CodecError::bad_input(e.to_string()))
    }
}

impl Unary for HyperClient {
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

        // The macro builds a path-only URI; resolve it against the base authority.
        let uri: Uri = format!("{}{}", self.base_url, parts.uri)
            .parse()
            .map_err(|e: http::uri::InvalidUri| ClientError::Encode(e.to_string()))?;

        let mut builder = Request::builder().method(parts.method).uri(uri);

        if let Some(headers) = builder.headers_mut() {
            *headers = parts.headers;
        }

        let request = builder
            .body(Full::new(Bytes::from(bytes)))
            .map_err(|e| ClientError::Encode(e.to_string()))?;

        let response = self.client.request(request).await.map_err(net_err)?;
        let status = response.status();
        let headers = response.headers().clone();

        let body_bytes = response
            .into_body()
            .collect()
            .await
            .map_err(net_err)?
            .to_bytes()
            .to_vec();

        let decoded = self
            .decode(body_bytes)
            .map_err(|e| ClientError::Decode(e.to_string()))?;

        Ok(HttpResponse::new(status, headers, decoded))
    }
}

/// Maps a hyper network failure onto the transport arm of [`ClientError`].
fn net_err<T, E>(error: T) -> ClientError<E>
where
    T: std::fmt::Display,
{
    ClientError::Transport(overseerd_transport::Error::Io(std::io::Error::other(
        error.to_string(),
    )))
}
