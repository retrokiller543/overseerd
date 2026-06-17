# Plan: Streaming RPCs (gRPC-style)

## Goal

Support the four gRPC method kinds across the wire protocol, the handler/extractor
layer, and the `#[rpc]` macro:

| Kind           | Request | Response | `OperationKind` |
| -------------- | ------- | -------- | --------------- |
| Unary          | one     | one      | `Unary`         |
| Server stream  | one     | many     | `ServerStream`  |
| Client stream  | many    | one      | `ClientStream`  |
| Bidirectional  | many    | many     | `BidiStream`    |

`OperationKind` already encodes the taxonomy (`crates/core/src/descriptors/service/rpc.rs`).
Only `Unary` is served today; everything below is what it takes to light up the rest.

## Design principle: infer the kind from the signature

The kind is **structural**, so derive it from the handler signature — never a manual
annotation (unlike the dropped query/command label, which was body-level intent). The
macro inspects params and return type:

- **streamed output** → return type is `Result<impl Stream<Item = Result<T>>>`, or a
  named alias `ResponseStream<T>`.
- **streamed input** → a parameter extractor `Streaming<T>` (an inbound `Stream<Item =
  Result<T>>`), analogous to `Payload<T>` but multi-item.

```rust
async fn unary(Payload(r): Payload<Req>)        -> Result<Resp>                  // Unary
async fn server(Payload(r): Payload<Req>)        -> Result<ResponseStream<Resp>>  // ServerStream
async fn client(input: Streaming<Req>)           -> Result<Resp>                  // ClientStream
async fn bidi(input: Streaming<Req>)             -> Result<ResponseStream<Resp>>  // BidiStream
```

The macro maps {has `Streaming<T>` param} × {returns `ResponseStream<T>`} → the four
`OperationKind` variants and selects the matching dispatch adapter. No `#[rpc(...)]`
argument is needed.

## Workstreams

### 1. Wire protocol (`crates/transport/src/protocol`)

Today a call is exactly one `WireRequest` → one `WireResponse`, correlated by `CallId`.
Streaming = multiple frames sharing one `CallId`, so the protocol gains framed,
id-tagged stream messages:

```rust
enum WireMessage {
    Request(WireRequest),          // unary or the opening frame of a stream
    Response(WireResponse),        // unary reply
    StreamItem { id: CallId, payload: Vec<u8> },   // one item, either direction
    StreamEnd  { id: CallId },                      // half-close (no more items)
    StreamError{ id: CallId, message: String },     // terminal failure
}
```

Notes:
- `CallId` already tags every frame, so multiple in-flight streams multiplex over the
  single ordered byte stream. The length-prefixed framing (`codec.rs`) is unchanged;
  `MAX_FRAME_LEN` now bounds each *item*, not the whole response.
- Ordering per `CallId` is guaranteed by the transport being a single ordered stream.
  The shared write half (now an `Arc<Mutex<W>>`) already makes each frame write atomic,
  so concurrent streams interleave safely at frame boundaries.
- **Head-of-line blocking**: all streams share one TCP/Unix connection, so a large item
  on one stream delays others. Acceptable for v1; the real fix is QUIC (one transport
  stream per call), which the `Connection` trait doc already anticipates.

### 2. Transport traits (`crates/transport/src/transport.rs`)

`Respond` currently sends exactly one response and consumes `self`. Add a streaming
responder without breaking the unary path:

```rust
trait Respond {                  // unchanged: unary
    fn respond(self, outcome: CallResult) -> ...;
}

trait RespondStream {            // new: server → client items
    type Sink: ResponseSink;
    fn into_sink(self) -> Self::Sink;   // converts a responder into a multi-send sink
}

trait ResponseSink: Send {
    fn send(&mut self, item: CallResult) -> impl Future<Output = Result<()>> + Send;
    fn finish(self)                       -> impl Future<Output = Result<()>> + Send; // emits StreamEnd
}
```

For inbound streams, `IncomingCall` gains an optional request stream that the
connection feeds from `StreamItem`/`StreamEnd` frames carrying its `CallId`:

```rust
struct IncomingCall {
    path: String,
    payload: Vec<u8>,                          // opening frame (client-stream: first item or empty)
    requests: Option<mpsc::Receiver<Vec<u8>>>, // Some(..) for client/bidi streaming
}
```

The connection demuxes inbound frames by `CallId` to the right request channel — the
same pattern the (removed) UDP router used, now justified because it is real
multiplexing rather than faking connections.

### 3. Handler + extractor layer (`crates/core/src/extract.rs`)

- `Streaming<T>`: a `FromContext` extractor wrapping the per-call inbound `Receiver`,
  deserializing each item to `T`, yielding `impl Stream<Item = Result<T>>`.
- `ResponseStream<T>`: a return wrapper over `impl Stream<Item = Result<T>>`; the
  adapter serializes each item to a `StreamItem` frame and emits `StreamEnd` at the end
  (or `StreamError` if the stream yields `Err`).
- New `Handler` arity impls / dispatch adapters per kind:
  - `dispatch_with` (unary) — exists.
  - `dispatch_server_stream`, `dispatch_client_stream`, `dispatch_bidi` — each drives
    the appropriate sink/source. The `#[service]`-generated wrapper picks the adapter
    based on the inferred `OperationKind`.

### 4. Serve loop (`crates/core/src/daemon.rs`)

`serve_connection` currently does recv → dispatch → respond, strictly sequential. For
streaming it must:
- spawn the handler future so the connection can keep reading inbound `StreamItem`
  frames for in-flight client/bidi calls while a handler runs;
- route inbound stream frames to the correct call's request channel by `CallId`;
- propagate cancellation: dropping the connection (or a per-call cancel) ends both the
  inbound channel and the outbound sink. A per-call `CancellationToken` in
  `RpcCallContext` lets handlers observe client-side cancellation.

This is the largest behavioral change: the loop goes from one-call-at-a-time to
concurrent calls multiplexed over the connection.

### 5. Macro (`crates/macros`)

- Infer `OperationKind` from the signature (rules above); drop the current
  "streaming not implemented" error in `attr::operation_variant`.
- Emit the matching dispatch adapter in the generated wrapper.
- Validate combinations (e.g. `Streaming<T>` must be the sole non-context param;
  reject `Payload<T>` + `Streaming<T>` together) with spanned errors.

## Phasing

1. **Taxonomy** — `OperationKind { Unary, ServerStream, ClientStream, BidiStream }`. ✅ done.
2. **Wire frames + multiplexing** — add stream messages; per-`CallId` demux on the
   connection; concurrent dispatch in `serve_connection`. Foundation for all streaming.
3. **Server streaming** — `ResponseStream<T>` + `ResponseSink`; most common, smallest
   surface (no inbound channel). Ship first.
4. **Client streaming** — `Streaming<T>` extractor + inbound request channel.
5. **Bidirectional** — compose 3 + 4; mostly falls out once both halves exist.
6. **Macro inference** — wire signature → kind → adapter; remove the temporary error.

## Cross-cutting concerns

- **Backpressure**: bounded channels both directions; `ResponseSink::send` awaits when
  the peer is slow. Matches the project's "prefer channels, locks at the lowest layer"
  discipline.
- **Cancellation/teardown**: a dropped connection closes every call's channels and
  sinks; add a per-call cancel token surfaced via `RpcCallContext`.
- **Errors**: a handler/stream error becomes a terminal `StreamError` frame; the client
  surfaces it as the stream's final `Err`.
- **Backward compatibility**: unary stays one `Request`→one `Response`; the new frames
  are additive, so existing unary clients/servers are unaffected.

## Testing

- `MemoryTransport` gains item/end/error plumbing for fast, deterministic stream tests.
- Per kind: happy path, mid-stream error, client cancellation, backpressure (slow
  consumer), and interleaving of two concurrent streams on one connection.

## Open questions

- `ResponseStream<T>` as a concrete wrapper vs. accepting any `impl Stream` via a return
  trait — the wrapper is simpler for the macro to detect and keeps coherence clean.
- Whether to add per-call flow-control windows now or defer until QUIC.
