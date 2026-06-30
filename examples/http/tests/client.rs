//! End-to-end test of the **generated HTTP client**: build the app, serve it on an ephemeral
//! port, and call it through the macro-generated `{Controller}Client` over the `ReqwestClient`
//! backend — exercising path params, a JSON body, and the `HttpResponse` envelope (status +
//! the deref-to-body), then shut the server down so the test never hangs.

use futures::{Stream, StreamExt};
use overseerd::axum::Ndjson;
use overseerd::axum::axum::Json;
use overseerd::axum::axum::extract::Path;
use overseerd::axum::client::ReqwestClient;
use overseerd::axum::prelude::*;
use overseerd::prelude::*;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

#[derive(Serialize, Deserialize)]
struct EchoOut {
    msg: String,
    len: usize,
}

#[derive(Serialize, Deserialize)]
struct SumIn {
    a: i64,
    b: i64,
}

/// A domain error the handler streams as part of its items (not an HTTP/transport error).
#[derive(Serialize, Deserialize, PartialEq, Debug)]
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

    #[post("/sum")]
    async fn sum(&self, Json(input): Json<SumIn>) -> Json<i64> {
        Json(input.a + input.b)
    }

    /// Two path params: the client exposes them as dedicated named args (`a`, `b`).
    #[get("/pair/{a}/{b}")]
    async fn pair(&self, Path((a, b)): Path<(i64, i64)>) -> Json<i64> {
        Json(a * b)
    }

    /// Pattern 1 — infallible items: the client mirrors `impl Stream<Item = u64>`, fallible only
    /// in the outer pre-stream `Result`. The macro injects `use<>` so the `&self` return compiles.
    #[get("/ticks/{n}", streamed)]
    async fn ticks(&self, Path(n): Path<u64>) -> Ndjson<impl Stream<Item = u64>> {
        Ndjson(futures::stream::iter(0..n))
    }

    /// Pattern 2 — fallible items (`Result<u64, ItemError>`): a mid-stream domain error surfaces
    /// as an `Err` *item*; the stream continues.
    #[get("/fallible", streamed)]
    async fn fallible(&self) -> Ndjson<impl Stream<Item = Result<u64, ItemError>>> {
        Ndjson(futures::stream::iter(vec![
            Ok(1),
            Ok(2),
            Err(ItemError {
                reason: "boom".into(),
            }),
            Ok(4),
        ]))
    }

    /// Pattern 3 — outer `Result` (pre-stream failure): the handler may fail before streaming;
    /// the client surfaces that as the outer call's `Err`, items stay infallible `u64`.
    #[get("/maybe/{ok}", streamed)]
    async fn maybe(
        &self,
        Path(ok): Path<u64>,
    ) -> Result<Ndjson<impl Stream<Item = u64>>, http::StatusCode> {
        if ok == 0 {
            return Err(http::StatusCode::IM_A_TEAPOT);
        }

        Ok(Ndjson(futures::stream::iter(0..ok)))
    }

    /// Pattern 4 — both: a pre-stream failure (outer `Result`) and fallible items.
    #[get("/both/{ok}", streamed)]
    async fn both(
        &self,
        Path(ok): Path<u64>,
    ) -> Result<Ndjson<impl Stream<Item = Result<u64, ItemError>>>, http::StatusCode> {
        if ok == 0 {
            return Err(http::StatusCode::IM_A_TEAPOT);
        }

        Ok(Ndjson(futures::stream::iter(vec![
            Ok(1),
            Err(ItemError {
                reason: "boom".into(),
            }),
        ])))
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

    // JSON-body route: `POST /api/sum`. The client takes the raw `SumIn`, not `Json<SumIn>` —
    // the wrapping happens inside the generated request builder.
    let summed = client.sum(SumIn { a: 2, b: 40 }).await.expect("sum call");
    assert_eq!(*summed, 42);

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

    shutdown.shutdown();
    let _ = server.await;
}
