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
///
/// It is exported to JS (a wasm-bindgen class), so a browser client builds an upload the natural way:
/// `text` takes strings and `file` takes a native [`File`]/[`Blob`] (from a file input, drag-drop,
/// or a fetched response), reading its bytes, name, and MIME type for you — no manual `Uint8Array`.
/// The byte-level `file` overload stays for native Rust callers, which have no `File`/`Blob` type.
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
    ///
    /// This byte-level form is for native Rust callers. The browser client instead uses the
    /// [`File`/`Blob`](Self::file) overload of this method (same JS name, `File`/`Blob` argument).
    #[cfg(not(all(target_family = "wasm", feature = "reqwest")))]
    pub fn file(&mut self, name: String, filename: String, content_type: String, data: Vec<u8>) {
        self.parts.push(MultipartPart {
            name,
            filename: Some(filename),
            content_type: Some(content_type),
            data,
        });
    }

    /// Adds a file upload from a native JS [`File`] or [`Blob`] — the ergonomic browser form.
    ///
    /// A `File` (e.g. `input.files[0]`) carries its own name and MIME type, so
    /// `await mp.file("avatar", file)` is enough. A bare `Blob` has no name — pass one as
    /// `filename`. An explicit `filename` always overrides a `File`'s own. A typeless blob defaults
    /// to `application/octet-stream`, matching `FormData`. Reading the blob's bytes is an async
    /// browser operation, so this method is `async` (callers `await` the add) — unlike
    /// [`text`](Self::text).
    #[cfg(all(target_family = "wasm", feature = "reqwest"))]
    pub async fn file(
        &mut self,
        name: String,
        blob: web_sys::Blob,
        filename: Option<String>,
    ) -> Result<(), ::wasm_bindgen::JsError> {
        self.push_blob(name, blob, filename).await
    }

    /// Adds several file uploads under one field name — the multi-file form (e.g. a list of avatars).
    ///
    /// Each `File`/`Blob` becomes its own part carrying the same `name`, in the order given, which is
    /// exactly how an HTML `<input type="file" multiple>` submits. Names/types are derived per file
    /// as in [`file`](Self::file); pass a real JS array (`Array.from(input.files)` or `[...files]`)
    /// since a `FileList` is array-like, not an `Array`.
    #[cfg(all(target_family = "wasm", feature = "reqwest"))]
    pub async fn files(
        &mut self,
        name: String,
        files: Vec<web_sys::Blob>,
    ) -> Result<(), ::wasm_bindgen::JsError> {
        for blob in files {
            self.push_blob(name.clone(), blob, None).await?;
        }

        Ok(())
    }
}

/// The browser upload path: read a JS [`Blob`]/[`File`]'s bytes and record it as a part. Private
/// (not exported to JS) and shared by [`Multipart::file`] and [`Multipart::files`].
#[cfg(all(target_family = "wasm", feature = "reqwest"))]
impl Multipart {
    async fn push_blob(
        &mut self,
        name: String,
        blob: web_sys::Blob,
        filename: Option<String>,
    ) -> Result<(), ::wasm_bindgen::JsError> {
        use ::wasm_bindgen::JsCast;

        let ty = blob.type_();

        let filename =
            filename.or_else(|| blob.dyn_ref::<web_sys::File>().map(web_sys::File::name));
        let content_type = if ty.is_empty() {
            "application/octet-stream".to_string()
        } else {
            ty
        };

        let buffer = ::wasm_bindgen_futures::JsFuture::from(blob.array_buffer())
            .await
            .map_err(|e| ::wasm_bindgen::JsError::new(&format!("failed to read blob: {e:?}")))?;
        let data = ::js_sys::Uint8Array::new(&buffer).to_vec();

        self.parts.push(MultipartPart {
            name,
            filename,
            content_type: Some(content_type),
            data,
        });

        Ok(())
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
