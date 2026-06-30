//! The generated HTTP client's runtime: the request body family, the response envelope, and
//! pluggable transport backends.
//!
//! A generated `{Controller}Client<C>` is transport-generic: `C` is any backend implementing
//! the [`Unary`](overseerd_client::Unary) capability with `Request<B> = http::Request<B>` and
//! `Response<R> = HttpResponse<R>` (plus the [`Encodes`](overseerd_transport::Encodes) /
//! [`Decodes`](overseerd_transport::Decodes) codec). Both bundled backends — [`reqwest`] and
//! `hyper` — qualify, so the same client runs over either; pick one with the matching feature.

mod body;
mod response;
mod streaming;

#[cfg(feature = "hyper")]
mod hyper_backend;
#[cfg(feature = "reqwest")]
mod reqwest_backend;

pub use body::{HttpBody, OctetStream};
pub use response::HttpResponse;
pub use streaming::{HttpClientStreaming, HttpStreaming, StreamDecode, encode_stream};

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

/// Maps a non-success HTTP response into a [`ClientError::Remote`](overseerd_client::ClientError),
/// carrying the genuine [`http::StatusCode`](axum::http::StatusCode) and the raw error body. The
/// HTTP client's protocol status *is* the HTTP status — no folding into the RPC packed status — so
/// a caller inspects `error.code()` as an `http::StatusCode` directly. Used by the streaming
/// transports, where a pre-stream failure has no response envelope to surface the status on (it is
/// the outer `Result`'s `Err`).
#[cfg(any(feature = "reqwest", feature = "hyper"))]
pub(crate) fn remote_error(
    status: axum::http::StatusCode,
    body: Vec<u8>,
) -> overseerd_client::ClientError<axum::http::StatusCode> {
    overseerd_client::ClientError::Remote(overseerd_client::ErrorBody::new(status, body))
}

#[cfg(feature = "hyper")]
pub use hyper_backend::HyperClient;
#[cfg(feature = "reqwest")]
pub use reqwest_backend::ReqwestClient;
