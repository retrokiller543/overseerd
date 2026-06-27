//! End-to-end tests for the request-flow middleware (TODO #11), driven over the
//! in-memory transport. Covers a tower middleware observing a call before and
//! after the handler, a `Guard` short-circuiting an unauthorized call, and a
//! global `ErrorHandler` remapping an outgoing error.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

use overseerd::tower::{Layer, Service};
use overseerd::{
    App, CallResult, ErrorHandler, ErrorResponse, Guard, MemoryClient, MemoryConnectionHandle,
    Payload, PredefinedCode, RpcAppBuilder, RpcCallContext, RpcOutcome, RpcRequest, StatusCode,
    handlers, service,
};

// ---------------------------------------------------------------------------
// A trivial service: one infallible rpc and one always-failing rpc.
// ---------------------------------------------------------------------------

/// Service exercising the middleware path.
#[service(id = "mw_svc", version = "0.1")]
struct MwSvc;

#[handlers]
impl MwSvc {
    /// Echoes its input back unchanged.
    #[rpc]
    async fn echo(Payload(n): Payload<u32>) -> u32 {
        n
    }

    /// Always returns a framework error (mapped to `BadInput`).
    #[rpc]
    async fn boom() -> overseerd::Result<u32> {
        Err(overseerd::Error::InvalidPayload("boom".to_string()))
    }
}

// ---------------------------------------------------------------------------
// A counting middleware: a tower Layer/Service that bumps a shared counter
// once before the handler and once after, proving it wraps the whole call.
// ---------------------------------------------------------------------------

/// Layer that installs a [`CountService`] counting calls before and after.
#[derive(Clone)]
struct CountLayer {
    calls: Arc<AtomicUsize>,
}

impl<S> Layer<S> for CountLayer {
    type Service = CountService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CountService {
            calls: Arc::clone(&self.calls),
            inner,
        }
    }
}

/// Service produced by [`CountLayer`].
#[derive(Clone)]
struct CountService<S> {
    calls: Arc<AtomicUsize>,
    inner: S,
}

impl<S> Service<RpcRequest> for CountService<S>
where
    S: Service<RpcRequest, Response = RpcOutcome, Error = ErrorResponse> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = RpcOutcome;
    type Error = ErrorResponse;
    type Future = Pin<Box<dyn Future<Output = Result<RpcOutcome, ErrorResponse>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: RpcRequest) -> Self::Future {
        let calls = Arc::clone(&self.calls);
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);

            let outcome = inner.call(req).await;

            calls.fetch_add(1, Ordering::SeqCst);

            outcome
        })
    }
}

// ---------------------------------------------------------------------------
// A guard that admits only even payloads, rejecting the rest as Unauthorized.
// ---------------------------------------------------------------------------

/// Rejects any `echo` call whose `u32` payload is odd.
struct EvenGuard;

impl Guard for EvenGuard {
    fn check<'a>(
        &'a self,
        ctx: &'a RpcCallContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), ErrorResponse>> + Send + 'a>> {
        Box::pin(async move {
            let value: u32 = postcard::from_bytes(&ctx.payload).unwrap_or_default();

            if value.is_multiple_of(2) {
                return Ok(());
            }

            Err(ErrorResponse::new(
                StatusCode::from(PredefinedCode::Unauthorized),
                Vec::new(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// A global error handler that remaps every error to a fixed code + body.
// ---------------------------------------------------------------------------

/// Replaces every outgoing error with `Unauthorized` and a marker body.
struct RemapHandler;

impl ErrorHandler for RemapHandler {
    fn handle<'a>(
        &'a self,
        _path: &'a str,
        _error: ErrorResponse,
    ) -> Pin<Box<dyn Future<Output = ErrorResponse> + Send + 'a>> {
        Box::pin(async move {
            ErrorResponse::new(
                StatusCode::from(PredefinedCode::Unauthorized),
                b"handled".to_vec(),
            )
        })
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Builds and serves a daemon configured by `configure`, returning a connected
/// client handle.
async fn start<F>(configure: F) -> MemoryConnectionHandle
where
    F: FnOnce(overseerd::AppBuilder) -> overseerd::AppBuilder,
{
    let (client, transport) = MemoryClient::pair();

    let builder = App::builder("test").auto_discover();
    let daemon = configure(builder).build().await.expect("build daemon");

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn middleware_wraps_call_before_and_after() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);

    let conn = start(move |b| b.middleware(CountLayer { calls: observed })).await;

    let result = conn.call("MwSvc.echo", enc(&7u32)).await.unwrap();

    match result {
        CallResult::Ok(body) => assert_eq!(dec::<u32>(&body), 7),

        other => panic!("expected ok, got {other:?}"),
    }

    // One increment before the handler, one after — the middleware saw both ends.
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn guard_admits_even_and_rejects_odd() {
    let conn = start(|b| b.guard(EvenGuard)).await;

    let allowed = conn.call("MwSvc.echo", enc(&4u32)).await.unwrap();

    match allowed {
        CallResult::Ok(body) => assert_eq!(dec::<u32>(&body), 4),

        other => panic!("expected ok for even input, got {other:?}"),
    }

    let rejected = conn.call("MwSvc.echo", enc(&5u32)).await.unwrap();

    match rejected {
        CallResult::Err { code, .. } => {
            assert_eq!(code.predefined(), PredefinedCode::Unauthorized);
        }

        other => panic!("expected unauthorized for odd input, got {other:?}"),
    }
}

#[tokio::test]
async fn error_handler_remaps_outgoing_error() {
    let conn = start(|b| b.error_handler(RemapHandler)).await;

    // `boom` returns a framework BadInput error; the global handler remaps it.
    let result = conn.call("MwSvc.boom", enc(&())).await.unwrap();

    match result {
        CallResult::Err { code, body } => {
            assert_eq!(code.predefined(), PredefinedCode::Unauthorized);
            assert_eq!(body, b"handled");
        }

        other => panic!("expected a remapped error, got {other:?}"),
    }
}
