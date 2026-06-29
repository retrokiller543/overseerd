//! End-to-end tests for the axum protocol: build the app, take its assembled `axum::Router`,
//! and drive real requests through it with `tower::ServiceExt::oneshot` — exercising the
//! per-request scope layer, the `Inject` DI extractor (a singleton, a request-scoped
//! component, and a resolver-backed `Cfg<T>`), and native axum extractors, with no live server.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use http_body_util::BodyExt;
use overseerd::axum::Ndjson;
use overseerd::axum::axum::body::Body;
use overseerd::axum::axum::extract::Path;
use overseerd::axum::axum::http::{Request, StatusCode};
use overseerd::axum::axum::{Json, Router};
use overseerd::axum::prelude::*;
use overseerd::prelude::*;
use overseerd::{component, config, methods};
use serde::{Deserialize, Serialize};
use tower::ServiceExt;

#[config(path = "greeting")]
#[derive(Serialize, Deserialize)]
struct GreetingConfig {
    #[default = "Hi"]
    salutation: String,
}

/// A shared singleton counter, field-injected into the controller.
#[component(by_value)]
#[derive(Clone)]
struct Counter {
    #[default]
    hits: Arc<AtomicU64>,
}

/// A per-request component, reachable only via route-level `Inject`.
#[component(scope = Request)]
struct Ticket {
    #[default]
    id: u64,
}

#[methods]
impl Ticket {
    #[init]
    async fn init() -> Self {
        Self { id: 42 }
    }
}

#[derive(Serialize)]
struct Reply {
    text: String,
    hits: u64,
}

#[controller(path = "/api")]
struct TestController {
    counter: Counter,
}

#[handlers]
impl TestController {
    /// Native `Path` extractor + `&self` singleton state.
    #[get("/hello/{who}")]
    async fn hello(&self, Path(who): Path<String>) -> Json<Reply> {
        let hits = self.counter.hits.fetch_add(1, Ordering::Relaxed) + 1;

        Json(Reply {
            text: format!("hello {who}"),
            hits,
        })
    }

    /// `Inject<Cfg<T>>` — the case that previously failed to compile (config is `FromContainer`,
    /// not `Injectable`). Resolves the config default through the request scope's resolvers.
    #[get("/cfg")]
    async fn cfg(&self, Inject(cfg): Inject<Cfg<GreetingConfig>>) -> Json<String> {
        Json(cfg.snapshot().salutation.clone())
    }

    /// `Inject` of a request-scoped component a singleton controller could not hold.
    #[get("/ticket")]
    async fn ticket(&self, Inject(ticket): Inject<Arc<Ticket>>) -> Json<u64> {
        Json(ticket.id)
    }

    /// Server-streaming: returns a streamed NDJSON body. `streamed` marks it for the client; the
    /// framing is the `Ndjson` wrapper the handler returns, not a macro-chosen format. The server
    /// side is just an `IntoResponse`.
    #[get("/count/{n}", streamed)]
    async fn count(
        &self,
        Path(n): Path<u64>,
    ) -> Ndjson<futures::stream::Iter<std::ops::Range<u64>>> {
        Ndjson(futures::stream::iter(0..n))
    }
}

/// Builds the app and returns its assembled router.
async fn router() -> Router {
    let app = app! {
        name: "test-http",
        protocol: overseerd::axum::AxumPlugin,
    }
    .build()
    .await
    .expect("app builds");

    app.protocol().router().clone()
}

/// Reads a response body to a UTF-8 string.
async fn body_string(response: overseerd::axum::axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();

    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn native_extractor_and_singleton_injection() {
    let router = router().await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/hello/world")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_string(response).await,
        r#"{"text":"hello world","hits":1}"#
    );
}

#[tokio::test]
async fn cfg_injection_resolves_default() {
    let router = router().await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/cfg")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    // The `#[default = "Hi"]` is seeded even without a config file, and resolves through the
    // request scope's inherited config resolver.
    assert_eq!(body_string(response).await, r#""Hi""#);
}

#[tokio::test]
async fn request_scoped_injection() {
    let router = router().await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/ticket")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_string(response).await, "42");
}

#[tokio::test]
async fn server_streaming_ndjson_body() {
    let router = router().await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/count/3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/x-ndjson")
    );
    // Three items, each its own JSON line.
    assert_eq!(body_string(response).await, "0\n1\n2\n");
}
