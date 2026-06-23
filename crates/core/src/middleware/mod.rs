//! Request-flow middleware built on [`tower`].
//!
//! The dispatch path is modelled as a [`tower::Service`] from an [`RpcRequest`]
//! to an [`RpcOutcome`] (or an [`ErrorResponse`]). [`RouterService`] is the
//! terminal service that looks the call up and invokes its handler; user
//! middleware are ordinary [`tower::Layer`]s wrapping it, registered on the
//! [`DaemonBuilder`](crate::daemon::DaemonBuilder). Because the request and
//! response types are concrete, any protocol-agnostic tower layer (timeout,
//! rate-limit, concurrency-limit, …) composes directly alongside framework ones.
//!
//! Two framework conveniences sit on top of the same mechanism:
//! [`Guard`] — a pre-handler admit/reject check adapted into a layer — and
//! [`ErrorHandler`], a single global hook applied to every error on its way to
//! the wire (including mid-stream errors), wired in the daemon runtime rather
//! than as a layer so it observes errors the layer stack never returns.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tower::util::BoxCloneService;
use tower::{Layer, Service};

use crate::descriptors::{RpcCallContext, RpcOutcome};
use crate::extract::ErrorResponse;
use crate::router::RpcRouter;

/// The unit of work flowing through the middleware stack: a call's request scope
/// and payload (`ctx`) plus the route `path` the terminal [`RouterService`] needs
/// to resolve the handler.
pub struct RpcRequest {
    pub path: String,
    pub ctx: RpcCallContext,
}

impl RpcRequest {
    /// Pairs a resolved route `path` with its call context.
    pub fn new(path: String, ctx: RpcCallContext) -> Self {
        Self { path, ctx }
    }
}

/// The type-erased, cloneable dispatch service stored on a built daemon: the
/// [`RouterService`] wrapped by every registered middleware layer.
pub type RpcService = BoxCloneService<RpcRequest, RpcOutcome, ErrorResponse>;

/// The terminal service of the middleware stack: resolves the request's `path`
/// against the router and invokes the matched handler.
#[derive(Clone)]
pub struct RouterService {
    router: Arc<RpcRouter>,
}

impl RouterService {
    /// Wraps a shared router as the innermost dispatch service.
    pub fn new(router: Arc<RpcRouter>) -> Self {
        Self { router }
    }
}

impl Service<RpcRequest> for RouterService {
    type Response = RpcOutcome;
    type Error = ErrorResponse;
    type Future = Pin<Box<dyn Future<Output = Result<RpcOutcome, ErrorResponse>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: RpcRequest) -> Self::Future {
        let router = Arc::clone(&self.router);

        Box::pin(async move { router.dispatch(&req.path, req.ctx).await })
    }
}

/// A pre-handler check that admits or rejects a call before it reaches the
/// handler. Returning `Err` short-circuits dispatch with that error response;
/// returning `Ok(())` lets the call proceed. Adapted into the middleware stack by
/// [`DaemonBuilder::guard`](crate::daemon::DaemonBuilder::guard).
pub trait Guard: Send + Sync + 'static {
    /// Inspects the call context and decides whether the call may proceed.
    fn check<'a>(
        &'a self,
        ctx: &'a RpcCallContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), ErrorResponse>> + Send + 'a>>;
}

/// A [`tower::Layer`] that runs a [`Guard`] before delegating to the inner
/// service. Constructed for you by `DaemonBuilder::guard`.
pub struct GuardLayer {
    guard: Arc<dyn Guard>,
}

impl GuardLayer {
    /// Wraps a shared guard as a layer.
    pub fn new(guard: Arc<dyn Guard>) -> Self {
        Self { guard }
    }
}

impl<S> Layer<S> for GuardLayer {
    type Service = GuardService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GuardService {
            guard: Arc::clone(&self.guard),
            inner,
        }
    }
}

/// The service produced by [`GuardLayer`]: runs the guard, then the inner service.
#[derive(Clone)]
pub struct GuardService<S> {
    guard: Arc<dyn Guard>,
    inner: S,
}

impl<S> Service<RpcRequest> for GuardService<S>
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
        let guard = Arc::clone(&self.guard);

        // Move the readied clone into the future and leave a fresh (not-yet-ready)
        // clone behind, per tower's poll_ready/call contract.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            guard.check(&req.ctx).await?;

            inner.call(req).await
        })
    }
}

/// A single global hook applied to every [`ErrorResponse`] on its way to the
/// caller — for centralized logging, enrichment, or remapping (a
/// `GlobalExceptionHandler` equivalent).
///
/// It composes with per-error [`ResponseError`](crate::extract::ResponseError):
/// `ResponseError` produces the `ErrorResponse` at the handler boundary, and the
/// handler is the last-mile transform of whatever error reaches the runtime —
/// including errors raised by middleware layers and errors emitted mid-stream.
pub trait ErrorHandler: Send + Sync + 'static {
    /// Transforms the outgoing error for the call at `path`.
    fn handle<'a>(
        &'a self,
        path: &'a str,
        error: ErrorResponse,
    ) -> Pin<Box<dyn Future<Output = ErrorResponse> + Send + 'a>>;
}
