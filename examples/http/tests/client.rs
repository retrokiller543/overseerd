//! End-to-end test of the **generated HTTP client**: build the app, serve it on an ephemeral
//! port, and call it through the macro-generated `{Controller}Client` over the `ReqwestClient`
//! backend — exercising path params, a JSON body, and the `HttpResponse` envelope (status +
//! the deref-to-body), then shut the server down so the test never hangs.

use futures::{Stream, StreamExt};
use overseerd::axum::Ndjson;
use overseerd::axum::axum::extract::Path;
use overseerd::axum::axum::{Json, http};
use overseerd::axum::client::{HyperClient, ReqwestClient};
use overseerd::axum::prelude::*;
use overseerd::client::{ClientError, Unary};
use overseerd::prelude::*;
use tokio::net::TcpListener;

#[dto]
struct EchoOut {
    msg: String,
    len: usize,
}

#[dto]
struct SumIn {
    a: i64,
    b: i64,
}

/// A domain error the handler streams as part of its items (not an HTTP/transport error).
#[dto]
#[derive(PartialEq, Debug)]
struct ItemError {
    reason: String,
}

/// A controller with a path-param route and a JSON-body route.
#[controller(path = "/api")]
struct Api {
    #[default]
    _unit: (),
}

#[handlers]
impl Api {
    #[get("/echo/{msg}")]
    async fn echo(&self, Path(msg): Path<String>) -> Json<EchoOut> {
        let len = msg.len();

        Json(EchoOut { msg, len })
    }

    #[get("/missing")]
    async fn missing(&self) -> http::StatusCode {
        http::StatusCode::NOT_FOUND
    }

    #[post("/sum")]
    async fn sum(&self, Json(input): Json<SumIn>) -> Json<i64> {
        Json(input.a + input.b)
    }

    /// Two path params: the client exposes them as dedicated named args (`a`, `b`).
    #[get("/pair/{a}/{b}")]
    async fn pair(&self, Path((a, b)): Path<(i64, i64)>) -> Json<i64> {
        Json(a * b)
    }

    /// Pattern 1 — inferred framing, infallible items: a bare `impl Stream<Item = u64>` (no
    /// wrapper, no `streamed` flag) is the shorthand for NDJSON. The macro wraps `Ndjson`
    /// server-side and injects `use<>` so the `&self` return compiles; the client mirrors
    /// `impl Stream<Item = u64>`.
    #[get("/ticks/{n}")]
    async fn ticks(&self, Path(n): Path<u64>) -> impl Stream<Item = u64> {
        futures::stream::iter(0..n)
    }

    /// Pattern 2 — inferred framing, fallible items (`Result<u64, ItemError>`): a mid-stream
    /// domain error surfaces as an `Err` *item*; the stream continues.
    #[get("/fallible")]
    async fn fallible(&self) -> impl Stream<Item = Result<u64, ItemError>> {
        futures::stream::iter(vec![
            Ok(1),
            Ok(2),
            Err(ItemError {
                reason: "boom".into(),
            }),
            Ok(4),
        ])
    }

    /// Pattern 3 — explicit `Ndjson` wrapper inside an outer `Result` (pre-stream failure): a
    /// known wrapper needs no `streamed` flag; the client surfaces the failure as the outer
    /// call's `Err`, items stay infallible `u64`.
    #[get("/maybe/{ok}")]
    async fn maybe(
        &self,
        Path(ok): Path<u64>,
    ) -> Result<Ndjson<impl Stream<Item = u64>>, http::StatusCode> {
        if ok == 0 {
            return Err(http::StatusCode::IM_A_TEAPOT);
        }

        Ok(Ndjson(futures::stream::iter(0..ok)))
    }

    /// Pattern 4 — both: a pre-stream failure (outer `Result`) and fallible items, via the
    /// inferred framing inside the `Result`.
    #[get("/both/{ok}")]
    async fn both(
        &self,
        Path(ok): Path<u64>,
    ) -> Result<impl Stream<Item = Result<u64, ItemError>>, http::StatusCode> {
        if ok == 0 {
            return Err(http::StatusCode::IM_A_TEAPOT);
        }

        Ok(futures::stream::iter(vec![
            Ok(1),
            Err(ItemError {
                reason: "boom".into(),
            }),
        ]))
    }

    /// Client-streaming: the client sends a stream of items as the request body; `#[stream]` marks
    /// the body parameter, the server reads it (axum body streaming) and returns one response.
    #[post("/collect")]
    async fn collect(&self, #[stream] items: impl Stream<Item = u64>) -> Json<u64> {
        let sum = items
            .fold(0u64, |acc, item| async move { acc + item })
            .await;

        Json(sum)
    }
}

#[tokio::test]
async fn generated_client_round_trips_over_reqwest() {
    let app = app! {
        name: "client-test",
        protocol: overseerd::axum::AxumPlugin,
    }
    .build()
    .await
    .expect("app builds");

    // Bind an ephemeral port, then serve on a background task so the test can issue requests
    // and shut the server down deterministically.
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(async move { app.serve(listener).await });

    // The generated client over the reqwest backend, pointed at the bound address.
    let client = ApiClient::new(ReqwestClient::new(format!("http://{addr}")));

    // Path-param route: `GET /api/echo/{msg}`. The response envelope derefs to the body.
    let echoed = client.echo("hello".to_string()).await.expect("echo call");
    assert_eq!(echoed.status().as_u16(), 200);
    assert_eq!(echoed.msg, "hello");
    assert_eq!(echoed.len, 5);

    let encoded = "hello/world ?#å".to_string();
    let echoed = client
        .echo(encoded.clone())
        .await
        .expect("encoded echo call");
    assert_eq!(echoed.msg, encoded);

    // JSON-body route: `POST /api/sum`. The client takes the raw `SumIn`, not `Json<SumIn>` —
    // the wrapping happens inside the generated request builder.
    let summed = client.sum(SumIn { a: 2, b: 40 }).await.expect("sum call");
    assert_eq!(*summed, 42);

    let backend = ReqwestClient::new(format!("http://{addr}"));
    let request = http::Request::builder()
        .method(http::Method::GET)
        .uri("/api/missing")
        .body(())
        .expect("request");

    match Unary::unary::<(), EchoOut, overseerd::client::Raw>(&backend, "", request).await {
        Err(ClientError::Remote(error)) => {
            // The HTTP client surfaces the genuine `http::StatusCode` — no folding into the RPC
            // packed status.
            assert_eq!(error.code(), http::StatusCode::NOT_FOUND);
            assert_eq!(error.code().as_u16(), 404);
        }

        Ok(_) => panic!("expected remote 404, got success"),
        Err(other) => panic!("expected remote 404, got {other:?}"),
    }

    // Two path params surface as dedicated named args: `GET /api/pair/{a}/{b}`.
    let product = client.pair(6, 7).await.expect("pair call");
    assert_eq!(*product, 42);

    // Pattern 1 — infallible items: the client mirrors `Stream<Item = u64>` (no per-item
    // `Result`); only the outer call is fallible. The NDJSON framing never appears in the type.
    let items: Vec<u64> = client.ticks(4).await.expect("ticks call").collect().await;
    assert_eq!(items, vec![0, 1, 2, 3]);

    // Pattern 2 — fallible items: the client mirrors `Stream<Item = Result<u64, ItemError>>`, so
    // a mid-stream domain error is an `Err` item and the stream keeps going.
    let items: Vec<Result<u64, ItemError>> = client
        .fallible()
        .await
        .expect("fallible call")
        .collect()
        .await;
    assert_eq!(items.len(), 4);
    assert_eq!(items[0], Ok(1));
    assert_eq!(
        items[2],
        Err(ItemError {
            reason: "boom".into()
        })
    );
    assert_eq!(items[3], Ok(4));

    // Pattern 3 — outer `Result`: a pre-stream failure surfaces as the outer call's `Err`, while
    // the happy path streams infallible items.
    assert!(
        client.maybe(0).await.is_err(),
        "pre-stream failure is the outer Err"
    );

    let items: Vec<u64> = client
        .maybe(3)
        .await
        .expect("maybe(3) streams")
        .collect()
        .await;
    assert_eq!(items, vec![0, 1, 2]);

    // Pattern 4 — both: pre-stream `Err`, or a stream of fallible items.
    assert!(
        client.both(0).await.is_err(),
        "pre-stream failure is the outer Err"
    );

    let items: Vec<Result<u64, ItemError>> = client
        .both(2)
        .await
        .expect("both(2) streams")
        .collect()
        .await;
    assert_eq!(items.len(), 2);
    assert_eq!(items[0], Ok(1));
    assert!(items[1].is_err());

    // Client-streaming: the client sends a stream of `u64` as the request body; the server folds
    // them and returns one response.
    let total = client
        .collect(futures::stream::iter(vec![1u64, 2, 3, 4]))
        .await
        .expect("collect call");
    assert_eq!(*total, 10);

    let hyper = ApiClient::new(HyperClient::new(format!("http://{addr}")));
    let total = hyper
        .collect(futures::stream::iter(vec![10u64, 20, 30]))
        .await
        .expect("hyper collect call");
    assert_eq!(*total, 60);

    shutdown.shutdown();
    let _ = server.await;
}
