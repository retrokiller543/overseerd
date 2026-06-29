//! End-to-end test of the **generated HTTP client**: build the app, serve it on an ephemeral
//! port, and call it through the macro-generated `{Controller}Client` over the `ReqwestClient`
//! backend — exercising path params, a JSON body, and the `HttpResponse` envelope (status +
//! the deref-to-body), then shut the server down so the test never hangs.

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

    // JSON-body route: `POST /api/sum`.
    let summed = client
        .sum(Json(SumIn { a: 2, b: 40 }))
        .await
        .expect("sum call");
    assert_eq!(*summed, 42);

    shutdown.shutdown();
    let _ = server.await;
}
