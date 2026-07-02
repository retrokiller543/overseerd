use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::get;
use tower::ServiceExt;

use super::{AxumMiddleware, as_layer};

struct CountingMiddleware {
    calls: Arc<AtomicUsize>,
}

impl AxumMiddleware for CountingMiddleware {
    async fn handle(&self, req: Request, next: Next) -> Response {
        self.calls.fetch_add(1, Ordering::SeqCst);

        next.run(req).await
    }
}

struct RejectingMiddleware;

impl AxumMiddleware for RejectingMiddleware {
    async fn handle(&self, _req: Request, _next: Next) -> Response {
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::empty())
            .unwrap()
    }
}

#[tokio::test]
async fn passthrough_middleware_runs_then_calls_handler() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mw = Arc::new(CountingMiddleware {
        calls: Arc::clone(&calls),
    });

    let router = axum::Router::new()
        .route("/", get(|| async { "ok" }))
        .layer(as_layer(mw));

    let response = router
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn short_circuiting_middleware_never_reaches_the_handler() {
    let router = axum::Router::new()
        .route("/", get(|| async { "ok" }))
        .layer(as_layer(Arc::new(RejectingMiddleware)));

    let response = router
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
