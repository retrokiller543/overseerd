//! DI-backed middleware: an additive, optional path alongside plain `tower`/axum middleware.
//!
//! `AxumAppBuilder::layer` takes any standard `tower::Layer` directly — `tower-http`, a
//! hand-written `axum::middleware::from_fn`, anything already written against axum's own
//! attach points keeps working unmodified. [`AxumMiddleware`] only adds value on top of that
//! for the specific case where the middleware itself wants dependency injection: a `#[component]`
//! implementing it is resolved once (as a DI singleton) and shared across every attach point
//! (global, controller, path) it's registered at, instead of being constructed per attach point.

use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::Route;
use overseerd_app::AppRuntime;
use tower::{Layer, Service};

/// DI-backed middleware: intercepts a request before/after axum's handler dispatch.
///
/// Short-circuiting (auth rejection, etc.) is just not calling `next.run(req)` and returning a
/// response directly.
pub trait AxumMiddleware: Send + Sync + 'static {
    /// Handles the request, either continuing via `next` or short-circuiting with a response.
    fn handle(&self, req: Request, next: Next) -> impl Future<Output = Response> + Send;
}

/// Adapts a resolved middleware singleton into a plain `tower::Layer`, so it composes with
/// axum's own `Router::layer`/`route_layer`/`MethodRouter::layer` alongside any standard
/// tower/axum middleware registered the same way.
pub fn as_layer<M>(
    mw: Arc<M>,
) -> impl Layer<
    Route,
    Service: Service<Request, Response = Response, Error = Infallible, Future: Send + 'static>
                 + Clone
                 + Send
                 + Sync
                 + 'static,
> + Clone
+ Send
+ Sync
+ 'static
where
    M: AxumMiddleware,
{
    middleware::from_fn::<_, (Request,)>(move |req: Request, next: Next| {
        let mw = Arc::clone(&mw);

        async move { mw.handle(req, next).await }
    })
}

/// One registered global attach-point action: applies a layer — raw or DI-resolved — to the
/// router being built. Boxed because it may capture a runtime layer *value* (not just a type),
/// mirroring the RPC protocol's `LayerApplier` (`overseerd_rpc::middleware`).
pub(crate) type MiddlewareApplier =
    Box<dyn FnOnce(&AppRuntime, axum::Router) -> axum::Router + Send>;

#[cfg(test)]
mod tests;
