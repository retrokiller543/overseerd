//! Request bodies: a typed wrapper owns its wire format and its `Content-Type`.

use overseerd_transport::CodecError;
use serde::Serialize;

/// The fixed `multipart/form-data` boundary the client encoder uses. Fixed (rather than random) so
/// [`Multipart`]'s `Content-Type` can stay an associated `&'static` const like every other body; the
/// encoder validates that no part contains it, erroring rather than producing a malformed payload.
macro_rules! multipart_boundary {
    () => {
        "overseerdFormBoundary7MA4YWxkTrZu0gW"
    };
}

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

/// A raw octet-stream body, for a format without a typed wrapper — pairs with the server's `Bytes`
/// extractor. The generated client takes the raw `Vec<u8>` and wraps it in this internally.
pub struct OctetStream(pub Vec<u8>);

impl HttpBody for OctetStream {
    const CONTENT_TYPE: Option<&'static str> = Some("application/octet-stream");

    fn encode(self) -> Result<Vec<u8>, CodecError> {
        Ok(self.0)
    }
}

/// A raw `application/x-www-form-urlencoded` body — pairs with the server's `RawForm` extractor, for
/// a pre-encoded (or non-`Serialize`) form payload. The generated client takes the raw `Vec<u8>` and
/// sends it verbatim under the form content type.
pub struct RawForm(pub Vec<u8>);

impl HttpBody for RawForm {
    const CONTENT_TYPE: Option<&'static str> = Some("application/x-www-form-urlencoded");

    fn encode(self) -> Result<Vec<u8>, CodecError> {
        Ok(self.0)
    }
}

/// One field of a [`Multipart`] body: a form field name, the raw bytes, and — for a file upload —
/// the reported filename and content type.
struct MultipartPart {
    name: String,
    filename: Option<String>,
    content_type: Option<String>,
    data: Vec<u8>,
}

/// A `multipart/form-data` request body — pairs with the server's `Multipart` extractor.
///
/// Built up field by field ([`text`](Self::text) / [`file`](Self::file)), then encoded with a fixed
/// boundary so it slots into the same [`HttpBody`] machinery as every other body (its `Content-Type`
/// can be a `&'static` const). This is a plain byte payload the transport sends like any other, so it
/// works identically on native and over the browser `fetch` backend — no `FormData` bridge needed.
/// It is exported to JS (a wasm-bindgen class), so a browser client builds an upload the same way.
#[cfg_attr(
    all(target_family = "wasm", feature = "reqwest"),
    ::wasm_bindgen::prelude::wasm_bindgen
)]
#[derive(Default)]
pub struct Multipart {
    parts: Vec<MultipartPart>,
}

#[cfg_attr(
    all(target_family = "wasm", feature = "reqwest"),
    ::wasm_bindgen::prelude::wasm_bindgen
)]
impl Multipart {
    /// An empty multipart body to add fields to.
    #[cfg_attr(
        all(target_family = "wasm", feature = "reqwest"),
        ::wasm_bindgen::prelude::wasm_bindgen(constructor)
    )]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a plain text form field.
    pub fn text(&mut self, name: String, value: String) {
        self.parts.push(MultipartPart {
            name,
            filename: None,
            content_type: None,
            data: value.into_bytes(),
        });
    }

    /// Adds a binary field (a file upload): `name` is the form field, `filename` the reported file
    /// name, `content_type` the part's MIME type, and `data` the raw bytes.
    pub fn file(&mut self, name: String, filename: String, content_type: String, data: Vec<u8>) {
        self.parts.push(MultipartPart {
            name,
            filename: Some(filename),
            content_type: Some(content_type),
            data,
        });
    }
}

impl HttpBody for Multipart {
    const CONTENT_TYPE: Option<&'static str> = Some(concat!(
        "multipart/form-data; boundary=",
        multipart_boundary!()
    ));

    fn encode(self) -> Result<Vec<u8>, CodecError> {
        let boundary = multipart_boundary!().as_bytes();

        let mut out = Vec::new();

        for part in &self.parts {
            // A fixed boundary is only sound if it never appears in a part's bytes; a collision would
            // corrupt the framing, so refuse rather than emit a malformed body.
            if part.data.windows(boundary.len()).any(|w| w == boundary) {
                return Err(CodecError::internal(
                    "multipart field data contains the encoder's boundary delimiter".to_string(),
                ));
            }

            out.extend_from_slice(b"--");
            out.extend_from_slice(boundary);
            out.extend_from_slice(b"\r\nContent-Disposition: form-data; name=\"");
            out.extend_from_slice(part.name.as_bytes());
            out.push(b'"');

            if let Some(filename) = &part.filename {
                out.extend_from_slice(b"; filename=\"");
                out.extend_from_slice(filename.as_bytes());
                out.push(b'"');
            }

            out.extend_from_slice(b"\r\n");

            if let Some(content_type) = &part.content_type {
                out.extend_from_slice(b"Content-Type: ");
                out.extend_from_slice(content_type.as_bytes());
                out.extend_from_slice(b"\r\n");
            }

            out.extend_from_slice(b"\r\n");
            out.extend_from_slice(&part.data);
            out.extend_from_slice(b"\r\n");
        }

        out.extend_from_slice(b"--");
        out.extend_from_slice(boundary);
        out.extend_from_slice(b"--\r\n");

        Ok(out)
    }
}
