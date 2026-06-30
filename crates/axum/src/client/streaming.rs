//! Client-side streaming: the byte-stream transport seam and the pluggable item *decoders*.
//!
//! A server-streaming response is one HTTP body carrying many items. The transport
//! ([`HttpStreaming`]) exposes that body as a stream of raw [`Bytes`] chunks; the framing
//! ([`StreamDecode`]) turns those chunks back into typed items. The generated client method
//! glues them and returns `impl Stream<Item = Result<T, ClientError>>` — the wire framing never
//! appears in its signature, mirroring the RPC client. A new framing is just another
//! [`StreamDecode`] impl, so nothing is hard-wired.

use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt};
use overseerd_client::ClientError;
use overseerd_transport::Encodes;
use serde::de::DeserializeOwned;

use crate::stream::{Ndjson, RawStream};

/// A transport that can open a streamed HTTP response: it sends `request` and yields the
/// response body as raw [`Bytes`] chunks. Implemented by the `reqwest` and `hyper` backends, so
/// a generated streaming client is transport-generic.
pub trait HttpStreaming: Send + Sync {
    /// The byte-chunk stream of a streamed response body.
    type ByteStream: Stream<Item = Result<Bytes, ClientError>> + Send + Unpin + 'static;

    fn open_stream<B>(
        &self,
        request: http::Request<B>,
    ) -> impl std::future::Future<Output = Result<Self::ByteStream, ClientError>> + Send
    where
        Self: Encodes<B>,
        B: Send;
}

/// Deframes a stream of raw body chunks into typed items, per a wire framing. Pluggable: a new
/// framing (multipart, length-delimited, …) is another impl. Keyed on the item type like
/// [`Encodes`](overseerd_transport::Encodes), so a framing supports exactly the items it carries.
///
/// The yielded item is the **exact** type the server declared (`T`, or a `Result<T, E>` the
/// handler chose to stream) — the client mirrors the server's types. A transport or frame-decode
/// failure is *not* surfaced as an item: it ends the stream with a logged warning (matching the
/// RPC inbound stream), so mid-stream errors only ever appear when the server's item type makes
/// them explicit. Pre-stream failures are the outer `Result` the generated method returns.
pub trait StreamDecode<T> {
    /// Turns a byte-chunk stream into a stream of decoded items.
    fn decode_stream<S>(body: S) -> impl Stream<Item = T> + Send
    where
        S: Stream<Item = Result<Bytes, ClientError>> + Send + Unpin + 'static;
}

/// NDJSON decoding: buffer bytes, split on `\n`, decode each line as JSON — across arbitrary
/// chunk boundaries. A transport or JSON error ends the stream with a logged warning.
impl<W, T> StreamDecode<T> for Ndjson<W>
where
    T: DeserializeOwned + Send + 'static,
{
    fn decode_stream<S>(body: S) -> impl Stream<Item = T> + Send
    where
        S: Stream<Item = Result<Bytes, ClientError>> + Send + Unpin + 'static,
    {
        // State threaded through `unfold`: the body, a pending buffer, and whether the body is
        // drained (so a trailing unterminated line is still decoded once).
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
                    // A complete line, or the trailing line once the body is drained.
                    let line = if let Some(newline) = state.buffer.iter().position(|&b| b == b'\n')
                    {
                        let line = state.buffer.split_to(newline);
                        let _ = state.buffer.split_to(1);

                        if line.is_empty() {
                            continue;
                        }

                        line
                    } else if state.done {
                        if state.buffer.is_empty() {
                            return None;
                        }

                        state.buffer.split()
                    } else {
                        match state.body.next().await {
                            Some(Ok(chunk)) => {
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

                    // Decode the line into `T`, or end the stream (logged) on a JSON error.
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
}

/// Raw passthrough: each received chunk is one [`Bytes`] item; a transport error ends the stream
/// with a logged warning.
impl<W> StreamDecode<Bytes> for RawStream<W> {
    fn decode_stream<S>(body: S) -> impl Stream<Item = Bytes> + Send
    where
        S: Stream<Item = Result<Bytes, ClientError>> + Send + Unpin + 'static,
    {
        body.take_while(|chunk| {
            if let Err(error) = chunk {
                tracing::warn!(
                    target: "overseerd::axum",
                    %error,
                    "stream transport error; ending stream"
                );
            }

            std::future::ready(chunk.is_ok())
        })
        .filter_map(|chunk| std::future::ready(chunk.ok()))
    }
}
