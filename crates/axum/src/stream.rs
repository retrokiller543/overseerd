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

// `StreamBody` (the axum request extractor) and the `IntoResponse` framings are server-only; their
// imports are gated with them. The framing markers, encoders, and the NDJSON decode engine below
// are pure and compile on every target (the generated streaming client names them).
#[cfg(not(target_family = "wasm"))]
use std::pin::Pin;
#[cfg(not(target_family = "wasm"))]
use std::task::{Context, Poll};

#[cfg(not(target_family = "wasm"))]
use axum::body::Body;
#[cfg(not(target_family = "wasm"))]
use axum::extract::{FromRequest, Request};
#[cfg(not(target_family = "wasm"))]
use axum::http::header::CONTENT_TYPE;
#[cfg(not(target_family = "wasm"))]
use axum::response::{IntoResponse, Response};
use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt};
use overseerd_transport::CodecError;
use serde::Serialize;
use serde::de::DeserializeOwned;

const MAX_NDJSON_LINE_BYTES: usize = 1024 * 1024;

/// A newline-delimited-JSON streamed response (`application/x-ndjson`): each item of the wrapped
/// stream is serialized to one JSON line.
pub struct Ndjson<S>(pub S);

#[cfg(not(target_family = "wasm"))]
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

#[cfg(not(target_family = "wasm"))]
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

/// Decodes a byte-chunk stream into NDJSON items, buffering across chunk boundaries. A transport
/// or JSON error ends the stream with a logged warning (never surfaced as an item) — the shared
/// engine behind both the server [`StreamBody`] extractor and the client decoder. Generic over
/// the chunk error so it serves the server's `axum::Error` body and the client's `ClientError`.
pub(crate) fn ndjson_decode<S, E, T>(body: S) -> impl Stream<Item = T> + Send
where
    S: Stream<Item = Result<Bytes, E>> + Send + Unpin + 'static,
    E: std::fmt::Display,
    T: DeserializeOwned + Send + 'static,
{
    struct State<S> {
        body: S,
        buffer: BytesMut,
        done: bool,
    }

    futures::stream::unfold(
        State {
            body,
            buffer: BytesMut::new(),
            done: false,
        },
        |mut state| async move {
            loop {
                let line = if let Some(newline) = state.buffer.iter().position(|&b| b == b'\n') {
                    let line = state.buffer.split_to(newline);
                    let _ = state.buffer.split_to(1);

                    if line.is_empty() {
                        continue;
                    }

                    if line.len() > MAX_NDJSON_LINE_BYTES {
                        tracing::warn!(
                            target: "overseerd::axum",
                            len = line.len(),
                            limit = MAX_NDJSON_LINE_BYTES,
                            "stream item exceeded maximum NDJSON line length; ending stream"
                        );

                        return None;
                    }

                    line
                } else if state.done {
                    if state.buffer.is_empty() {
                        return None;
                    }

                    if state.buffer.len() > MAX_NDJSON_LINE_BYTES {
                        tracing::warn!(
                            target: "overseerd::axum",
                            len = state.buffer.len(),
                            limit = MAX_NDJSON_LINE_BYTES,
                            "stream item exceeded maximum NDJSON line length; ending stream"
                        );

                        return None;
                    }

                    state.buffer.split()
                } else {
                    match state.body.next().await {
                        Some(Ok(chunk)) => {
                            let pending_line_len = match chunk.iter().position(|&b| b == b'\n') {
                                Some(newline) => state.buffer.len() + newline,
                                None => state.buffer.len() + chunk.len(),
                            };

                            if pending_line_len > MAX_NDJSON_LINE_BYTES {
                                tracing::warn!(
                                    target: "overseerd::axum",
                                    len = pending_line_len,
                                    limit = MAX_NDJSON_LINE_BYTES,
                                    "stream item exceeded maximum NDJSON line length; ending stream"
                                );

                                return None;
                            }

                            state.buffer.extend_from_slice(&chunk);

                            continue;
                        }

                        Some(Err(error)) => {
                            tracing::warn!(
                                target: "overseerd::axum",
                                %error,
                                "stream transport error; ending stream"
                            );

                            return None;
                        }

                        None => {
                            state.done = true;

                            continue;
                        }
                    }
                };

                match serde_json::from_slice::<T>(&line) {
                    Ok(item) => return Some((item, state)),

                    Err(error) => {
                        tracing::warn!(
                            target: "overseerd::axum",
                            %error,
                            "stream item failed to decode; ending stream"
                        );

                        return None;
                    }
                }
            }
        },
    )
}

/// A streamed **request body**, deframed into items for a `#[stream]` handler parameter. The
/// server reads the request body through axum's streaming and yields `T` per NDJSON line (a
/// transport/decode error ends the stream, logged). A handler writes `#[stream] items: impl
/// Stream<Item = T>`; the macro extracts via this and hands the handler the inner stream.
#[cfg(not(target_family = "wasm"))]
pub struct StreamBody<T> {
    inner: Pin<Box<dyn Stream<Item = T> + Send>>,
}

#[cfg(not(target_family = "wasm"))]
impl<T> StreamBody<T> {
    /// The deframed item stream.
    pub fn into_stream(self) -> impl Stream<Item = T> + Send {
        self.inner
    }
}

#[cfg(not(target_family = "wasm"))]
impl<T> Stream for StreamBody<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        self.get_mut().inner.as_mut().poll_next(cx)
    }
}

#[cfg(not(target_family = "wasm"))]
impl<S, T> FromRequest<S> for StreamBody<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send + 'static,
{
    // Deframing degrades to an empty/short stream (logged) rather than rejecting, so a body that
    // is malformed mid-way still delivers the items decoded so far.
    type Rejection = std::convert::Infallible;

    async fn from_request(request: Request, _state: &S) -> Result<Self, Self::Rejection> {
        let body = request.into_body().into_data_stream();

        Ok(StreamBody {
            inner: Box::pin(ndjson_decode(body)),
        })
    }
}

/// Frames a stream of items into the bytes of a streamed body — the encode dual of
/// [`ndjson_decode`], used by the client to send a `#[stream]` request body. The framing wrapper
/// (`Ndjson`/`RawStream`/a custom one) picks the wire format; pluggable, never hard-wired.
pub trait StreamEncode<T> {
    /// The `Content-Type` a body framed this way carries.
    const CONTENT_TYPE: &'static str;

    /// Frames one item to its on-the-wire bytes (including any delimiter).
    fn encode(item: T) -> Result<Bytes, CodecError>;
}

/// NDJSON: one JSON value per line.
impl<W, T> StreamEncode<T> for Ndjson<W>
where
    T: Serialize,
{
    const CONTENT_TYPE: &'static str = "application/x-ndjson";

    fn encode(item: T) -> Result<Bytes, CodecError> {
        let mut bytes =
            serde_json::to_vec(&item).map_err(|e| CodecError::internal(e.to_string()))?;
        bytes.push(b'\n');

        Ok(Bytes::from(bytes))
    }
}

/// Raw passthrough: each `Bytes` item is sent unframed.
impl<W> StreamEncode<Bytes> for RawStream<W> {
    const CONTENT_TYPE: &'static str = "application/octet-stream";

    fn encode(item: Bytes) -> Result<Bytes, CodecError> {
        Ok(item)
    }
}

/// Coalesces a `Stream<Item = u8>` into ready-batched `Bytes` chunks, so a byte stream maps onto
/// [`RawStream`] without a one-byte HTTP chunk per item. The controller macro inserts this when it
/// infers raw framing from a bare `impl Stream<Item = u8>` return.
pub fn chunk_u8<S>(stream: S) -> impl Stream<Item = Bytes>
where
    S: Stream<Item = u8> + Send + 'static,
{
    stream.ready_chunks(8192).map(Bytes::from)
}

#[cfg(test)]
mod tests;
