//! End-to-end tests for the generated client SDK, driven over a real byte pipe
//! (`tokio::io::duplex`) so the codec, `CallId` multiplexing, and stream
//! termination are all exercised — the parts the in-memory transport bypasses.
//!
//! A single service is served on one duplex connection; the generated
//! `CalcClient` (a trait-form client) drives it. The assertions double as the
//! codegen smoke test: they pin the return-type extraction rules (`Result<T, E>`
//! decoding to `T` with the error typed as `E::Body`, `Option<T>` left intact,
//! and each streaming kind).

#![cfg(feature = "client")]

use serde::{Deserialize, Serialize};
use tokio::io::{DuplexStream, ReadHalf, WriteHalf};

use overseer::{
    ClientConnection, ClientError, Daemon, ErrorResponse, Payload, ResponseError, ResponseStream,
    StreamClientTransport, Streaming, handlers, service,
    transport::{PeerInfo, StreamConnection, Transport},
};

// ---------------------------------------------------------------------------
// A service covering every return shape the client codegen must handle.
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct AddRequest {
    a: i32,
    b: i32,
}

/// The structured body a `CalcError` serializes — deliberately a different type
/// from the error itself, to prove the client recovers `E::Body`, not `E`.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct CalcErrorBody {
    reason: String,
}

/// A handler error whose serialized body is a distinct type.
#[derive(Debug)]
enum CalcError {
    Negative,
}

impl ResponseError for CalcError {
    type Body = CalcErrorBody;

    fn error_response(self) -> ErrorResponse {
        let body = CalcErrorBody {
            reason: "operands must be non-negative".to_string(),
        };

        ErrorResponse::with_serialized_body(self.status_code(), &body)
    }
}

/// Stateless calculator service. `client_trait = CalcApi` selects the
/// `dyn`-compatible trait form of the generated client.
#[service(id = "calc", version = "0.1")]
struct Calc;

#[handlers(client_trait = CalcApi)]
impl Calc {
    #[rpc]
    async fn ping() -> u32 {
        1
    }

    #[rpc]
    async fn add(Payload(req): Payload<AddRequest>) -> Result<i32, CalcError> {
        if req.a < 0 || req.b < 0 {
            return Err(CalcError::Negative);
        }

        Ok(req.a + req.b)
    }

    #[rpc]
    async fn maybe(Payload(present): Payload<bool>) -> Option<u32> {
        present.then_some(7)
    }

    #[rpc]
    async fn count(Payload(n): Payload<u32>) -> ResponseStream<u32> {
        ResponseStream::new(futures::stream::iter((0..n).map(Ok)))
    }

    #[rpc]
    async fn sum(mut input: Streaming<u32>) -> overseer::Result<u32> {
        let mut total = 0;

        while let Some(item) = input.next().await {
            total += item?;
        }

        Ok(total)
    }

    #[rpc]
    async fn echo(input: Streaming<u32>) -> ResponseStream<u32> {
        ResponseStream::new(input.map(|item| item.map(|v| v * 2)))
    }
}

use futures::StreamExt;

// ---------------------------------------------------------------------------
// Harness: serve one duplex connection, return a client over the other half.
// ---------------------------------------------------------------------------

type ServerConn = StreamConnection<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;
type Client = CalcClient<StreamClientTransport<WriteHalf<DuplexStream>>>;

/// A transport that yields exactly one pre-built connection, then never again.
struct OnceTransport {
    conn: Option<ServerConn>,
}

impl Transport for OnceTransport {
    type Connection = ServerConn;

    async fn accept(&mut self) -> overseer::transport::Result<Self::Connection> {
        match self.conn.take() {
            Some(conn) => Ok(conn),

            None => std::future::pending().await,
        }
    }
}

/// Builds the daemon, serves it over one half of a duplex pipe, and wraps the
/// other half in a generated client.
async fn start() -> Client {
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let (server_read, server_write) = tokio::io::split(server_io);
    let (client_read, client_write) = tokio::io::split(client_io);

    let daemon = Daemon::builder("test")
        .auto_discover()
        .build()
        .await
        .expect("build daemon");
    let server_conn = StreamConnection::new(server_read, server_write, PeerInfo { addr: None });

    tokio::spawn(async move {
        let _ = daemon
            .serve(OnceTransport {
                conn: Some(server_conn),
            })
            .await;
    });

    CalcClient::new(ClientConnection::new(StreamClientTransport::new(
        client_read,
        client_write,
    )))
}

// ---------------------------------------------------------------------------
// Unary
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unary_bare_value() {
    let client = start().await;

    assert_eq!(client.ping().await.expect("ping"), 1);
}

#[tokio::test]
async fn unary_result_ok() {
    let client = start().await;

    let sum = client
        .add(&AddRequest { a: 2, b: 3 })
        .await
        .expect("add");

    assert_eq!(sum, 5);
}

#[tokio::test]
async fn unary_result_err_decodes_body_type() {
    let client = start().await;

    // `add` returns `Result<i32, CalcError>`, but the wire body is a
    // `CalcErrorBody`; the generated client types the error as `E::Body`, so
    // `deserialize` yields the structured body directly.
    match client.add(&AddRequest { a: -1, b: 3 }).await {
        Err(ClientError::Remote(err)) => {
            let body = err.deserialize().expect("decode body");

            assert_eq!(
                body,
                CalcErrorBody {
                    reason: "operands must be non-negative".to_string(),
                }
            );
        }

        other => panic!("expected remote error, got {other:?}"),
    }
}

#[tokio::test]
async fn unary_option_is_not_peeled() {
    let client = start().await;

    assert_eq!(client.maybe(&true).await.expect("maybe"), Some(7));
    assert_eq!(client.maybe(&false).await.expect("maybe"), None);
}

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

#[tokio::test]
async fn server_stream_collects_items() {
    let client = start().await;

    let mut stream = client.count(&4u32).await.expect("count");
    let mut items = Vec::new();

    while let Some(item) = stream.next().await {
        items.push(item.expect("item"));
    }

    assert_eq!(items, vec![0, 1, 2, 3]);
}

#[tokio::test]
async fn client_stream_sums_inputs() {
    let client = start().await;

    let mut upstream = client.sum().await.expect("sum");

    for i in [1u32, 2, 3, 4] {
        upstream.send(&i).await.expect("send");
    }

    assert_eq!(upstream.finish().await.expect("finish"), 10);
}

#[tokio::test]
async fn bidi_echoes_doubled() {
    let client = start().await;

    let mut bidi = client.echo().await.expect("echo");

    bidi.send(&5u32).await.expect("send");
    assert_eq!(bidi.next().await.expect("item").expect("ok"), 10);

    bidi.send(&7u32).await.expect("send");
    assert_eq!(bidi.next().await.expect("item").expect("ok"), 14);

    bidi.close_send().await.expect("close");
}

// ---------------------------------------------------------------------------
// Concurrency: two calls multiplexed over one connection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn concurrent_calls_multiplex() {
    let client = start().await;

    let (a, b) = tokio::join!(
        client.add(&AddRequest { a: 1, b: 1 }),
        client.add(&AddRequest { a: 10, b: 10 }),
    );

    assert_eq!(a.expect("a"), 2);
    assert_eq!(b.expect("b"), 20);
}
