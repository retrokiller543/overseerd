//! The generated HTTP client's runtime: the request body family, the response envelope, and
//! pluggable transport backends.
//!
//! A generated `{Controller}Client<C>` is transport-generic: `C` is any backend implementing
//! the [`Unary`](overseerd_client::Unary) capability with `Request<B> = http::Request<B>` and
//! `Response<R> = HttpResponse<R>` (plus the [`Encodes`](overseerd_transport::Encodes) /
//! [`Decodes`](overseerd_transport::Decodes) codec). Both bundled backends — [`reqwest`] and
//! `hyper` — qualify, so the same client runs over either; pick one with the matching feature.

mod body;
// The shared browser-client `Connection` (wasm-only; needs the reqwest fetch backend).
#[cfg(all(target_family = "wasm", feature = "reqwest"))]
mod connection;
mod headers;
mod interceptor;
mod response;
// The STOMP client transport is cross-target (native + wasm) via `tokio-tungstenite-wasm`.
#[cfg(all(feature = "stomp", feature = "client"))]
mod stomp;
mod streaming;
// Shared ws runtime (task spawn) for the client transports; only needed with a ws transport.
#[cfg(all(feature = "tungstenite", feature = "client"))]
mod ws_rt;
// The JsonWs request/reply backend is native-only for now (its heart-beat uses a tokio timer).
#[cfg(all(feature = "ws", feature = "client", not(target_family = "wasm")))]
mod websocket;

#[cfg(all(feature = "hyper", not(target_family = "wasm")))]
mod hyper_backend;
#[cfg(feature = "reqwest")]
mod reqwest_backend;

pub use body::{Form, HttpBody, Json, Multipart, OctetStream, RawForm};
#[cfg(all(target_family = "wasm", feature = "reqwest"))]
pub use connection::Connection;
pub use headers::RequestHeaders;
#[cfg(target_family = "wasm")]
pub use interceptor::WasmClientInterceptor;
pub use interceptor::{ClientInterceptor, DefaultClientInterceptor};
pub use response::HttpResponse;
#[cfg(all(feature = "stomp", feature = "client"))]
pub use stomp::*;
pub use streaming::{HttpClientStreaming, HttpStreaming, StreamDecode, encode_stream};
#[cfg(all(feature = "ws", feature = "client", not(target_family = "wasm")))]
pub use websocket::*;

/// Re-exported so generated streaming-client code names the codec without a separate dep.
pub use overseerd_transport::{Decodes, Encodes};

/// Percent-encodes one URI path segment according to RFC 3986. Generated clients call this for
/// every route `Path<T>` substitution before building the request URI.
pub fn encode_path_segment(value: impl std::fmt::Display) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let value = value.to_string();
    let mut out = String::with_capacity(value.len());

    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }

    out
}

/// URL-encodes a typed `Query<T>` value into a query string (without the leading `?`). Generated
/// clients call this for a route's `Query<T>` param when building the request URI. Like path
/// substitution, it is a "valid by construction" step in the infallible URI builder: a `Dto` query
/// type serializes cleanly, so an encoder error (a shape `serde_urlencoded` rejects, e.g. a nested
/// struct) surfaces as a panic rather than a checked error on every call.
pub fn encode_query<T: serde::Serialize>(value: &T) -> String {
    serde_urlencoded::to_string(value).expect("query value serializes to a URL-encoded string")
}

/// Maps a non-success HTTP response into a [`ClientError::Remote`](overseerd_client::ClientError),
/// carrying the genuine [`http::StatusCode`](axum::http::StatusCode) and the raw error body. The
/// HTTP client's protocol status *is* the HTTP status — no folding into the RPC packed status — so
/// a caller inspects `error.code()` as an `http::StatusCode` directly. Used by the streaming
/// transports, where a pre-stream failure has no response envelope to surface the status on (it is
/// the outer `Result`'s `Err`).
#[cfg(any(feature = "reqwest", feature = "hyper"))]
pub(crate) fn remote_error(
    status: http::StatusCode,
    body: Vec<u8>,
) -> overseerd_client::ClientError<http::StatusCode> {
    overseerd_client::ClientError::Remote(overseerd_client::ErrorBody::new(status, body))
}

#[cfg(all(feature = "hyper", not(target_family = "wasm")))]
pub use hyper_backend::HyperClient;
#[cfg(feature = "reqwest")]
pub use reqwest_backend::ReqwestClient;
