//! Streamed response bodies — the streaming analogue of the unary body wrappers
//! (`Json<T>`/`Form<T>`).
//!
//! The wrapper *type* is the framing, never a separate format parameter: a handler returns
//! [`Ndjson<S>`] (newline-delimited JSON over the item stream `S`) or [`RawStream<S>`] (raw
//! `Bytes` chunks), and that one type picks both the encoding and the `Content-Type`. A new
//! framing — multipart, length-delimited, … — is just another wrapper that is an
//! [`IntoResponse`]; nothing is locked to one format.
//!
//! Because handlers are ordinary axum handlers, the server side needs no macro or capability
//! support: returning a wrapper *is* the response. (A bare `impl Stream` cannot be returned
//! directly — Rust's orphan rule blocks an `IntoResponse` impl for it — so the newtype is the
//! thinnest possible wrapper, and `Ndjson(stream)` reads no heavier than the stream itself.)

use axum::body::{Body, Bytes};
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use futures::{Stream, StreamExt};
use overseerd_transport::CodecError;
use serde::Serialize;

/// A newline-delimited-JSON streamed response (`application/x-ndjson`): each item of the wrapped
/// stream is serialized to one JSON line.
pub struct Ndjson<S>(pub S);

impl<S, T> IntoResponse for Ndjson<S>
where
    S: Stream<Item = T> + Send + 'static,
    T: Serialize + 'static,
{
    fn into_response(self) -> Response {
        let body = self.0.map(|item| {
            serde_json::to_vec(&item)
                .map(|mut bytes| {
                    bytes.push(b'\n');

                    Bytes::from(bytes)
                })
                .map_err(|e| CodecError::internal(e.to_string()))
        });

        (
            [(CONTENT_TYPE, "application/x-ndjson")],
            Body::from_stream(body),
        )
            .into_response()
    }
}

/// A raw byte-stream response (`application/octet-stream`): each [`Bytes`] chunk is written
/// through unframed — for a binary stream a typed framing does not fit.
pub struct RawStream<S>(pub S);

impl<S> IntoResponse for RawStream<S>
where
    S: Stream<Item = Bytes> + Send + 'static,
{
    fn into_response(self) -> Response {
        let body = self.0.map(Ok::<Bytes, std::convert::Infallible>);

        (
            [(CONTENT_TYPE, "application/octet-stream")],
            Body::from_stream(body),
        )
            .into_response()
    }
}
