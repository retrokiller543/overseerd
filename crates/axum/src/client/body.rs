//! Request bodies: a typed wrapper owns its wire format and its `Content-Type`.

use overseerd_transport::CodecError;
use serde::Serialize;

/// A typed request body that owns its content type and encoding.
///
/// The body's *type* picks the wire format and the `Content-Type` header that rides with it —
/// mirroring the server extractors, so a handler taking `Json<T>` pairs with a client sending
/// `Json<T>`, and form data is just `Form<T>`. [`CONTENT_TYPE`](Self::CONTENT_TYPE) is an
/// associated const so the generated client can set the header from the type alone, without an
/// instance.
pub trait HttpBody {
    /// The `Content-Type` this body sets, or `None` for an empty body.
    const CONTENT_TYPE: Option<&'static str>;

    /// Encodes the body to wire bytes.
    fn encode(self) -> Result<Vec<u8>, CodecError>;
}

/// An empty body — a `GET`/`DELETE` with nothing to send.
impl HttpBody for () {
    const CONTENT_TYPE: Option<&'static str> = None;

    fn encode(self) -> Result<Vec<u8>, CodecError> {
        Ok(Vec::new())
    }
}

/// A JSON request body — the common default, pairing with the server's `Json<T>` extractor.
///
/// Client-owned (not `axum::Json`) so the client body path carries no dependency on the axum
/// server framework and compiles for wasm. The generated client wraps the raw `T` in this
/// internally, so callers never name it — swapping the wrapper is invisible to them.
pub struct Json<T>(pub T);

impl<T: Serialize> HttpBody for Json<T> {
    const CONTENT_TYPE: Option<&'static str> = Some("application/json");

    fn encode(self) -> Result<Vec<u8>, CodecError> {
        serde_json::to_vec(&self.0).map_err(|e| CodecError::internal(e.to_string()))
    }
}

/// A URL-encoded form request body — pairs with the server's `Form<T>` extractor. Client-owned
/// (see [`Json`]) so it stays wasm-safe.
pub struct Form<T>(pub T);

impl<T: Serialize> HttpBody for Form<T> {
    const CONTENT_TYPE: Option<&'static str> = Some("application/x-www-form-urlencoded");

    fn encode(self) -> Result<Vec<u8>, CodecError> {
        serde_urlencoded::to_string(&self.0)
            .map(String::into_bytes)
            .map_err(|e| CodecError::internal(e.to_string()))
    }
}

/// A JSON body over the server's `axum::Json<T>` extractor type. Kept for native back-compat with
/// code that hands the axum wrapper directly to the client; the generated client uses [`Json`].
#[cfg(not(target_family = "wasm"))]
impl<T: Serialize> HttpBody for axum::Json<T> {
    const CONTENT_TYPE: Option<&'static str> = Some("application/json");

    fn encode(self) -> Result<Vec<u8>, CodecError> {
        serde_json::to_vec(&self.0).map_err(|e| CodecError::internal(e.to_string()))
    }
}

/// A URL-encoded form body over the server's `axum::extract::Form<T>` extractor type (native-only,
/// see the [`axum::Json`] impl above). The generated client uses [`Form`].
#[cfg(not(target_family = "wasm"))]
impl<T: Serialize> HttpBody for axum::extract::Form<T> {
    const CONTENT_TYPE: Option<&'static str> = Some("application/x-www-form-urlencoded");

    fn encode(self) -> Result<Vec<u8>, CodecError> {
        serde_urlencoded::to_string(&self.0)
            .map(String::into_bytes)
            .map_err(|e| CodecError::internal(e.to_string()))
    }
}

/// A raw octet-stream body, for a format without a typed wrapper.
pub struct OctetStream(pub Vec<u8>);

impl HttpBody for OctetStream {
    const CONTENT_TYPE: Option<&'static str> = Some("application/octet-stream");

    fn encode(self) -> Result<Vec<u8>, CodecError> {
        Ok(self.0)
    }
}
