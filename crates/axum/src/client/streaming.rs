//! Client-side streaming: the byte-stream transport seam and the pluggable item *decoders*.
//!
//! A server-streaming response is one HTTP body carrying many items. The transport
//! ([`HttpStreaming`]) exposes that body as a stream of raw [`Bytes`] chunks; the framing
//! ([`StreamDecode`]) turns those chunks back into typed items. The generated client method
//! glues them and returns `impl Stream<Item = Result<T, ClientError>>` — the wire framing never
//! appears in its signature, mirroring the RPC client. A new framing is just another
//! [`StreamDecode`] impl, so nothing is hard-wired.

use bytes::Bytes;
use futures::{Stream, StreamExt};
use overseerd_client::ClientError;
use overseerd_transport::{CodecError, Decodes, Encodes};
use serde::de::DeserializeOwned;

use crate::client::HttpResponse;
use crate::stream::{Ndjson, RawStream, StreamEncode};

/// Frames an item stream into the body chunks of a client-streaming request, per the framing `F`
/// (e.g. `Ndjson<()>`). Called by the generated client so the per-call `StreamExt::map` lives
/// here, not in emitted code.
pub fn encode_stream<F, T, S>(input: S) -> impl Stream<Item = Result<Bytes, CodecError>> + Send
where
    F: StreamEncode<T>,
    S: Stream<Item = T> + Send + 'static,
    T: Send + 'static,
{
    input.map(|item| <F as StreamEncode<T>>::encode(item))
}

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

/// A transport that can send a **streamed request body** and return the unary response: the
/// client-streaming dual of [`HttpStreaming`]. The request carries a stream of framed body chunks
/// (`http::Request<S>` with the frame stream as its body); the response is decoded like a unary
/// call (status + headers in the [`HttpResponse`] envelope). Implemented by `reqwest` and `hyper`.
pub trait HttpClientStreaming: Send + Sync {
    fn send_stream<S, Resp, E>(
        &self,
        request: http::Request<S>,
    ) -> impl std::future::Future<Output = Result<HttpResponse<Resp>, ClientError<E>>> + Send
    where
        Self: Decodes<Resp>,
        S: Stream<Item = Result<Bytes, CodecError>> + Send + 'static,
        Resp: Send;
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
        // The same buffering NDJSON engine the server's `StreamBody` extractor uses.
        crate::stream::ndjson_decode(body)
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
