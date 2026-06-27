//! End-to-end tests for the four RPC kinds and the `Responder` return path,
//! driven over the in-memory transport so they are fast and deterministic.
//!
//! All services in this binary are auto-discovered into one daemon, which is
//! served on a `MemoryTransport` in a background task; each test opens calls on
//! a fresh connection and asserts on the server's events.

use std::time::Duration;

use futures::{Stream, StreamExt};

use overseerd::{
    App, CallResult, Cancel, MemoryClient, MemoryConnectionHandle, Payload, ResponseStream,
    ServerEvent, StreamDecode, StreamDecodeError, StreamEncode, StreamEncodeError, Streaming,
    handlers, service,
};

// ---------------------------------------------------------------------------
// One service exercising every kind plus the Responder return variants.
// Stateless (no `&self`), so handlers are plain associated fns.
// ---------------------------------------------------------------------------

/// Test service covering all four RPC kinds and Responder return shapes.
#[service(id = "stream_svc", version = "0.1")]
struct StreamSvc;

#[handlers]
impl StreamSvc {
    // --- Responder return shapes (all unary) ---

    #[rpc]
    async fn bare() -> u32 {
        42
    }

    #[rpc]
    async fn unit() {}

    #[rpc]
    async fn maybe(Payload(present): Payload<bool>) -> Option<u32> {
        present.then_some(7)
    }

    #[rpc]
    async fn fallible_ok() -> overseerd::Result<u32> {
        Ok(1)
    }

    #[rpc]
    async fn fallible_err() -> overseerd::Result<u32> {
        Err(overseerd::Error::InvalidPayload("nope".to_string()))
    }

    // --- Server streaming: one request, many responses ---

    #[rpc]
    async fn count(Payload(n): Payload<u32>) -> ResponseStream<u32> {
        ResponseStream::new(futures::stream::iter((0..n).map(Ok)))
    }

    #[rpc]
    async fn fail_at_two() -> ResponseStream<u32> {
        ResponseStream::new(futures::stream::iter(vec![
            Ok(0),
            Ok(1),
            Err(overseerd::Error::InvalidPayload("boom".to_string())),
        ]))
    }

    #[rpc]
    async fn forever(cancel: Cancel) -> ResponseStream<u32> {
        let token = cancel.0;

        let stream = futures::stream::unfold(0u32, move |i| {
            let token = token.clone();

            async move {
                tokio::select! {
                    _ = token.cancelled() => None,
                    _ = tokio::time::sleep(Duration::from_millis(5)) => Some((Ok(i), i + 1)),
                }
            }
        });

        ResponseStream::new(stream)
    }

    // --- Client streaming: many requests, one response ---

    #[rpc]
    async fn sum(mut input: Streaming<u32>) -> overseerd::Result<u32> {
        let mut total = 0;

        while let Some(item) = input.next().await {
            total += item?;
        }

        Ok(total)
    }

    // --- Bidirectional: many requests, many responses ---

    #[rpc]
    async fn echo(input: Streaming<u32>) -> ResponseStream<u32> {
        ResponseStream::new(input.map(|item| item.map(|v| v * 2)))
    }
}

// ---------------------------------------------------------------------------
// A second service exercising the ergonomic stream forms: `impl Stream` and
// generic `S: Stream` parameters/returns (auto-detected, auto-wrapped), a
// concrete return marked `#[rpc(stream)]`, and a hand-rolled item codec.
// ---------------------------------------------------------------------------

/// A custom response item with a hand-rolled one-byte wire format. It is not
/// `Serialize`/`Deserialize`, so it uses its manual `StreamEncode`/`StreamDecode`
/// impls rather than the postcard blanket.
#[derive(Debug, PartialEq)]
struct Tag(u8);

impl StreamEncode for Tag {
    fn encode(&self) -> Result<Vec<u8>, StreamEncodeError> {
        Ok(vec![self.0])
    }
}

impl StreamDecode for Tag {
    fn decode(bytes: &[u8]) -> Result<Self, StreamDecodeError> {
        bytes
            .first()
            .copied()
            .map(Tag)
            .ok_or_else(|| StreamDecodeError("empty tag frame".to_string()))
    }
}

/// A custom error item terminating a fallible response stream.
#[derive(serde::Serialize, serde::Deserialize)]
struct StreamFault {
    reason: String,
}

/// Service covering the ergonomic stream forms.
#[service(id = "ergo_svc", version = "0.1")]
struct ErgoSvc;

#[handlers]
impl ErgoSvc {
    /// Server streaming via `impl Stream<Item = T>` (infallible items, no wrap).
    #[rpc]
    async fn count_up(Payload(n): Payload<u32>) -> impl Stream<Item = u32> {
        futures::stream::iter(0..n)
    }

    /// Server streaming via `impl Stream<Item = Result<T, E>>`: a custom error item
    /// terminates the stream with a `StreamError`.
    #[rpc]
    async fn fail_after(Payload(n): Payload<u32>) -> impl Stream<Item = Result<u32, StreamFault>> {
        futures::stream::iter((0..=n).map(move |i| {
            if i == n {
                Err(StreamFault {
                    reason: "stop".to_string(),
                })
            } else {
                Ok(i)
            }
        }))
    }

    /// Client streaming via a named generic `S: Stream<Item = T>` parameter.
    #[rpc]
    async fn sum_generic<S: futures::Stream<Item = u32>>(input: S) -> u32 {
        input.fold(0u32, |total, n| async move { total + n }).await
    }

    /// Bidirectional via `impl Stream` on both sides.
    #[rpc]
    async fn double(input: impl Stream<Item = u32>) -> impl Stream<Item = u32> {
        input.map(|v| v * 2)
    }

    /// Server streaming over a concrete stream type the macro cannot introspect,
    /// opted in with `#[rpc(stream)]`.
    #[rpc(stream)]
    async fn flagged(Payload(n): Payload<u32>) -> futures::stream::Iter<std::ops::Range<u32>> {
        futures::stream::iter(0..n)
    }

    /// Server streaming of a custom item using its hand-rolled wire codec.
    #[rpc]
    async fn tags(Payload(n): Payload<u8>) -> impl Stream<Item = Tag> {
        futures::stream::iter((0..n).map(Tag))
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Builds the daemon, serves it on a memory transport in the background, and
/// returns an open client connection.
async fn start() -> MemoryConnectionHandle {
    let (client, transport) = MemoryClient::pair();

    let daemon = App::builder("test")
        .auto_discover()
        .build()
        .await
        .expect("build daemon");

    tokio::spawn(async move {
        let _ = daemon.serve(transport).await;
    });

    client.connect().await.expect("connect")
}

fn enc<T: serde::Serialize>(value: &T) -> Vec<u8> {
    postcard::to_allocvec(value).unwrap()
}

fn dec<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> T {
    postcard::from_bytes(bytes).unwrap()
}

/// Drains a streaming call into its items, returning whether it ended cleanly
/// (`true` = `StreamEnd`, `false` = `StreamError`).
async fn drain(call: &mut overseerd::MemoryCall) -> (Vec<u32>, bool) {
    let mut items = Vec::new();

    loop {
        match call.recv().await {
            Some(ServerEvent::Item(bytes)) => items.push(dec::<u32>(&bytes)),
            Some(ServerEvent::End) => return (items, true),
            Some(ServerEvent::Error { .. }) => return (items, false),
            Some(ServerEvent::Response(_)) => panic!("unexpected unary response in a stream"),
            None => return (items, false),
        }
    }
}

// ---------------------------------------------------------------------------
// Macro inference: the kind is derived from the signature
// ---------------------------------------------------------------------------

#[tokio::test]
async fn infers_operation_kinds() {
    let daemon = App::builder("test")
        .auto_discover()
        .build()
        .await
        .expect("build daemon");

    let services = daemon.registry.resolved_services();
    let svc = services
        .iter()
        .find(|s| s.descriptor.name == "StreamSvc")
        .expect("StreamSvc registered");

    let kind = |name: &str| {
        let rpc = svc
            .rpcs
            .iter()
            .find(|r| r.name == name)
            .expect("rpc present");

        format!("{:?}", rpc.operation)
    };

    assert_eq!(kind("bare"), "Unary");
    assert_eq!(kind("fallible_ok"), "Unary");
    assert_eq!(kind("count"), "ServerStream");
    assert_eq!(kind("forever"), "ServerStream");
    assert_eq!(kind("sum"), "ClientStream");
    assert_eq!(kind("echo"), "BidiStream");
}

// ---------------------------------------------------------------------------
// Unary / Responder return shapes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn responder_shapes() {
    let conn = start().await;

    let bare = conn.call("StreamSvc.bare", enc(&())).await.unwrap();
    assert!(matches!(bare, CallResult::Ok(ref b) if dec::<u32>(b) == 42));

    // `()` serializes to an empty postcard body.
    let unit = conn.call("StreamSvc.unit", enc(&())).await.unwrap();
    assert!(matches!(unit, CallResult::Ok(ref b) if b.is_empty()));

    let some = conn.call("StreamSvc.maybe", enc(&true)).await.unwrap();
    assert!(matches!(some, CallResult::Ok(ref b) if dec::<Option<u32>>(b) == Some(7)));

    let none = conn.call("StreamSvc.maybe", enc(&false)).await.unwrap();
    assert!(matches!(none, CallResult::Ok(ref b) if dec::<Option<u32>>(b).is_none()));

    let ok = conn.call("StreamSvc.fallible_ok", enc(&())).await.unwrap();
    assert!(matches!(ok, CallResult::Ok(ref b) if dec::<u32>(b) == 1));

    let err = conn.call("StreamSvc.fallible_err", enc(&())).await.unwrap();
    assert!(matches!(err, CallResult::Err { .. }));
}

// ---------------------------------------------------------------------------
// Server streaming
// ---------------------------------------------------------------------------

#[tokio::test]
async fn server_stream_happy_path() {
    let conn = start().await;

    let mut call = conn
        .open("StreamSvc.count", enc(&4u32), false)
        .await
        .unwrap();
    let (items, clean) = drain(&mut call).await;

    assert!(clean);
    assert_eq!(items, vec![0, 1, 2, 3]);
}

#[tokio::test]
async fn server_stream_mid_stream_error() {
    let conn = start().await;

    let mut call = conn
        .open("StreamSvc.fail_at_two", enc(&()), false)
        .await
        .unwrap();
    let (items, clean) = drain(&mut call).await;

    // Items before the error are delivered, then the stream terminates as error.
    assert_eq!(items, vec![0, 1]);
    assert!(!clean);
}

#[tokio::test]
async fn server_stream_client_cancellation() {
    let conn = start().await;

    let mut call = conn
        .open("StreamSvc.forever", enc(&()), false)
        .await
        .unwrap();

    // Receive a couple of items, then cancel the call.
    assert!(matches!(call.recv().await, Some(ServerEvent::Item(_))));
    assert!(matches!(call.recv().await, Some(ServerEvent::Item(_))));

    call.cancel();

    // After cancellation the stream must terminate (draining any in-flight items).
    let mut ended = false;

    while let Some(event) = call.recv().await {
        match event {
            ServerEvent::Item(_) => continue,
            ServerEvent::End | ServerEvent::Error { .. } => {
                ended = true;
                break;
            }
            ServerEvent::Response(_) => panic!("unexpected unary response"),
        }
    }

    assert!(ended, "cancelled stream should terminate");
}

#[tokio::test]
async fn server_stream_backpressure_preserves_order() {
    let conn = start().await;

    // Capacity-1 event buffer forces the producer to await between items
    // (the backpressure path); all items must still arrive in order.
    let mut call = conn
        .open_with_capacity("StreamSvc.count", enc(&8u32), false, 1)
        .await
        .unwrap();

    let mut items = Vec::new();

    loop {
        match call.recv().await {
            Some(ServerEvent::Item(bytes)) => {
                items.push(dec::<u32>(&bytes));
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Some(ServerEvent::End) => break,
            other => panic!("unexpected event: {other:?}"),
        }
    }

    assert_eq!(items, (0..8).collect::<Vec<_>>());
}

// ---------------------------------------------------------------------------
// Client streaming
// ---------------------------------------------------------------------------

#[tokio::test]
async fn client_stream_sums_inputs() {
    let conn = start().await;

    let mut call = conn.open("StreamSvc.sum", Vec::new(), true).await.unwrap();

    for i in [1u32, 2, 3, 4] {
        call.send(enc(&i)).await.unwrap();
    }

    call.end_input();

    let out = call.response().await.unwrap();
    assert!(matches!(out, CallResult::Ok(ref b) if dec::<u32>(b) == 10));
}

// ---------------------------------------------------------------------------
// Bidirectional streaming
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bidi_echoes_doubled() {
    let conn = start().await;

    let mut call = conn.open("StreamSvc.echo", Vec::new(), true).await.unwrap();

    call.send(enc(&5u32)).await.unwrap();
    assert!(matches!(call.recv().await, Some(ServerEvent::Item(ref b)) if dec::<u32>(b) == 10));

    call.send(enc(&7u32)).await.unwrap();
    assert!(matches!(call.recv().await, Some(ServerEvent::Item(ref b)) if dec::<u32>(b) == 14));

    call.end_input();
    assert!(matches!(call.recv().await, Some(ServerEvent::End)));
}

// ---------------------------------------------------------------------------
// Concurrency: two streams interleaved on one connection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn concurrent_streams_on_one_connection() {
    let conn = start().await;

    let mut a = conn
        .open("StreamSvc.count", enc(&3u32), false)
        .await
        .unwrap();
    let mut b = conn
        .open("StreamSvc.count", enc(&5u32), false)
        .await
        .unwrap();

    let (items_a, items_b) = tokio::join!(drain(&mut a), drain(&mut b));

    assert_eq!(items_a, (vec![0, 1, 2], true));
    assert_eq!(items_b, (vec![0, 1, 2, 3, 4], true));
}

// ---------------------------------------------------------------------------
// Ergonomic stream forms (impl Stream / generic S: Stream / #[rpc(stream)])
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ergo_operation_kinds() {
    let daemon = App::builder("test")
        .auto_discover()
        .build()
        .await
        .expect("build daemon");

    let services = daemon.registry.resolved_services();
    let svc = services
        .iter()
        .find(|s| s.descriptor.name == "ErgoSvc")
        .expect("ErgoSvc registered");

    let kind = |name: &str| {
        let rpc = svc
            .rpcs
            .iter()
            .find(|r| r.name == name)
            .expect("rpc present");

        format!("{:?}", rpc.operation)
    };

    assert_eq!(kind("count_up"), "ServerStream");
    assert_eq!(kind("fail_after"), "ServerStream");
    assert_eq!(kind("sum_generic"), "ClientStream");
    assert_eq!(kind("double"), "BidiStream");
    assert_eq!(kind("flagged"), "ServerStream");
    assert_eq!(kind("tags"), "ServerStream");
}

#[tokio::test]
async fn impl_stream_server_stream() {
    let conn = start().await;

    let mut call = conn
        .open("ErgoSvc.count_up", enc(&4u32), false)
        .await
        .unwrap();
    let (items, clean) = drain(&mut call).await;

    assert!(clean);
    assert_eq!(items, vec![0, 1, 2, 3]);
}

#[tokio::test]
async fn impl_stream_fallible_terminates() {
    let conn = start().await;

    let mut call = conn
        .open("ErgoSvc.fail_after", enc(&2u32), false)
        .await
        .unwrap();
    let (items, clean) = drain(&mut call).await;

    // Items before the custom error are delivered, then the stream errors out.
    assert_eq!(items, vec![0, 1]);
    assert!(!clean);
}

#[tokio::test]
async fn generic_stream_client_stream() {
    let conn = start().await;

    let mut call = conn
        .open("ErgoSvc.sum_generic", Vec::new(), true)
        .await
        .unwrap();

    for i in [10u32, 20, 30] {
        call.send(enc(&i)).await.unwrap();
    }

    call.end_input();

    let out = call.response().await.unwrap();
    assert!(matches!(out, CallResult::Ok(ref b) if dec::<u32>(b) == 60));
}

#[tokio::test]
async fn impl_stream_bidi_doubles() {
    let conn = start().await;

    let mut call = conn.open("ErgoSvc.double", Vec::new(), true).await.unwrap();

    call.send(enc(&5u32)).await.unwrap();
    assert!(matches!(call.recv().await, Some(ServerEvent::Item(ref b)) if dec::<u32>(b) == 10));

    call.end_input();
    assert!(matches!(call.recv().await, Some(ServerEvent::End)));
}

#[tokio::test]
async fn flagged_concrete_server_stream() {
    let conn = start().await;

    let mut call = conn
        .open("ErgoSvc.flagged", enc(&3u32), false)
        .await
        .unwrap();
    let (items, clean) = drain(&mut call).await;

    assert!(clean);
    assert_eq!(items, vec![0, 1, 2]);
}

#[tokio::test]
async fn custom_item_codec_round_trip() {
    let conn = start().await;

    let mut call = conn.open("ErgoSvc.tags", enc(&3u8), false).await.unwrap();
    let mut items = Vec::new();

    loop {
        match call.recv().await {
            // The hand-rolled codec frames each `Tag` as a single byte.
            Some(ServerEvent::Item(bytes)) => items.push(Tag::decode(&bytes).unwrap()),
            Some(ServerEvent::End) => break,
            other => panic!("unexpected event: {other:?}"),
        }
    }

    assert_eq!(items, vec![Tag(0), Tag(1), Tag(2)]);
}
