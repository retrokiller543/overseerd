//! The pluggable protocol seam.
//!
//! A [`Protocol`] is a serve/dispatch mechanism layered over the agnostic
//! [`AppRuntime`]. The native RPC protocol is [`Rpc`]: it owns the router + middleware
//! stack and drives the per-connection / per-call loop over any
//! [`Transport`](overseerd_transport::Transport), opening connection and request scopes
//! through the runtime. The serve envelope (lifecycle hooks, reload triggers, ctrl-c) is
//! run by [`App`](crate::App) around [`Serve::serve`], so a protocol only watches its
//! endpoint and the shutdown signal.

use std::future::Future;
use std::sync::Arc;

use futures::StreamExt;
use overseerd_core::{Scope, TypeDescriptor};
use overseerd_di::{BoxedComponent, ScopeContainer};
use overseerd_transport::{
    CallResult, Connection, PeerInfo, Respond, RespondStream, ResponseSink, Transport,
};
use tokio::{sync::mpsc, task::JoinSet};
use tokio_util::sync::CancellationToken;
use tower::{Service, ServiceExt};
use tracing::{debug, error, info, instrument, warn};

use crate::descriptors::{RpcCallContext, RpcOutcome, RpcResponse};
use crate::extract::ErrorResponse;
use crate::lifecycle::ShutdownSignal;
use crate::middleware::{ErrorHandler, RpcRequest, RpcService};
use crate::registry::DescriptorRegistry;
use crate::router::RpcRouter;
use crate::runtime::AppRuntime;
use crate::scope::{Connection as ConnectionScope, Request as RequestScope};

/// A general extension unit applied to an app while it is built.
///
/// A plugin is the builder-time accumulator for an extension: it starts empty
/// ([`Default`]), gathers protocol-specific configuration through the builder, and
/// contributes DI descriptors (and, later, custom `#[component]` variants and their
/// discovery) into the registry before the container is built. A plugin need not serve
/// traffic — that is the job of the [`ProtocolPlugin`] sub-trait. Background behavior
/// rides the components a plugin registers (via their own `#[hook]`s).
pub trait Plugin: Default {
    /// Contributes DI descriptors / seeds into the registry before validation and build.
    /// The native RPC plugin seeds its connection-scoped `PeerInfo` here.
    fn register(&self, registry: &mut DescriptorRegistry);
}

/// A [`Plugin`] that additionally installs a serve/dispatch [`Protocol`]. An [`App`] is
/// built around exactly one of these.
pub trait ProtocolPlugin: Plugin {
    /// The protocol this plugin installs.
    type Protocol: Protocol;
    type Error: std::error::Error + Send + Sync + 'static;

    /// The session scope chain this protocol opens, root→leaf by rank, *excluding* the
    /// universal `Singleton` (root) and `Transient` (per-resolve). RPC opens
    /// `[Connection, Request]`; a request-only protocol opens `[Request]`.
    const SCOPES: &'static [&'static dyn Scope];

    /// Finalizes the protocol from the accumulated builder state, the assembled runtime,
    /// and the validated registry — for RPC, building the router and folding the
    /// middleware stack.
    fn build(
        self,
        runtime: &AppRuntime,
        registry: &DescriptorRegistry,
    ) -> Result<Self::Protocol, Self::Error>;
}

/// A pluggable serve/dispatch layer over the app's DI runtime. There is exactly one
/// active protocol per [`App`](crate::App).
pub trait Protocol: Send + 'static {
    type Error: std::error::Error + Send + Sync + 'static;
}

/// Serves a built protocol over a concrete endpoint type `E`. Kept separate from
/// [`Protocol`] so one protocol can serve many endpoint types — RPC over any
/// [`Transport`], a future HTTP protocol over a `SocketAddr`. The serve loop only needs
/// to watch `endpoint` and `shutdown`; lifecycle and reload are handled by the caller.
pub trait Serve<E>: Protocol {
    fn serve(
        self,
        runtime: AppRuntime,
        shutdown: ShutdownSignal,
        endpoint: E,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// The native RPC protocol: a router wrapped by the middleware stack, plus the global
/// error handler. Built by [`AppBuilder::build`](crate::AppBuilder::build) and served
/// over a [`Transport`].
pub struct Rpc {
    router: Arc<RpcRouter>,
    service: RpcService,
    error_handler: Option<Arc<dyn ErrorHandler>>,
    needs_peer: bool,
}

impl Rpc {
    pub(crate) fn new(
        router: Arc<RpcRouter>,
        service: RpcService,
        error_handler: Option<Arc<dyn ErrorHandler>>,
        needs_peer: bool,
    ) -> Self {
        Self {
            router,
            service,
            error_handler,
            needs_peer,
        }
    }

    /// The number of routes the dispatcher resolves.
    pub fn route_count(&self) -> usize {
        self.router.route_count()
    }
}

impl Protocol for Rpc {
    type Error = crate::Error;
}

impl<T> Serve<T> for Rpc
where
    T: Transport,
    T::Connection: 'static,
{
    async fn serve(
        self,
        runtime: AppRuntime,
        mut shutdown: ShutdownSignal,
        mut transport: T,
    ) -> crate::Result<()> {
        let transport_name = std::any::type_name::<T>();

        info!(target: "overseerd::daemon", app = runtime.name(), transport = transport_name, "serve starting");

        loop {
            tokio::select! {
                result = transport.accept() => {
                    match result {
                        Ok(conn) => {
                            debug!(target: "overseerd::daemon", peer = ?conn.peer().addr, "connection accepted, spawning task");

                            let service = self.service.clone();
                            let error_handler = self.error_handler.clone();
                            let needs_peer = self.needs_peer;
                            let runtime = runtime.clone();
                            tokio::spawn(async move {
                                serve_connection(conn, service, error_handler, needs_peer, runtime)
                                    .await;
                            });
                        }

                        Err(e) => {
                            error!(target: "overseerd::daemon", error = %e, "transport accept failed");
                            return Err(e.into());
                        }
                    }
                }

                _ = shutdown.wait() => {
                    info!(target: "overseerd::daemon", "shutdown signal received");
                    break;
                }
            }
        }

        info!(target: "overseerd::daemon", transport = transport_name, "serve stopped");

        Ok(())
    }
}

/// Serves one connection: builds the connection scope (seeding the peer when a
/// component depends on it), then drives each inbound call on its own task so
/// streaming calls run concurrently and the connection keeps reading inbound frames
/// while handlers run.
#[instrument(
    target = "overseerd::daemon",
    level = "debug",
    skip_all,
    fields(peer = ?conn.peer().addr),
    name = "connection"
)]
async fn serve_connection<C: Connection>(
    mut conn: C,
    service: RpcService,
    error_handler: Option<Arc<dyn ErrorHandler>>,
    needs_peer: bool,
    runtime: AppRuntime,
) {
    debug!(target: "overseerd::daemon", "connection established");

    // The peer (by value — the framework's connection-scoped injectable) is seeded
    // only when a component depends on it; handlers reach it through the `Peer`
    // extractor regardless, so an otherwise-empty connection scope is skipped. A
    // failed factory closes the connection.
    let peer = conn.peer().clone();
    let seeds = if needs_peer {
        vec![BoxedComponent {
            ty: TypeDescriptor::of::<PeerInfo>("PeerInfo"),
            value: Box::new(peer.clone()),
        }]
    } else {
        Vec::new()
    };

    let connection_scope = match runtime
        .open_scope(&ConnectionScope, Arc::clone(runtime.root()), seeds)
        .await
    {
        Ok(scope) => scope,

        Err(e) => {
            error!(target: "overseerd::daemon", error = %e, "connection scope build failed, closing");
            return;
        }
    };

    let mut tasks: JoinSet<()> = JoinSet::new();

    debug!(target: "overseerd::daemon", "connection ready");

    loop {
        match conn.recv().await {
            Ok(Some((call, responder))) => {
                let path = call.path;
                let service = service.clone();
                let error_handler = error_handler.clone();
                let connection_scope = Arc::clone(&connection_scope);
                let runtime = runtime.clone();
                let peer = peer.clone();

                debug!(target: "overseerd::daemon", %path, "dispatching call");

                tasks.spawn(drive_call(
                    path,
                    call.payload,
                    call.requests,
                    call.cancel,
                    peer,
                    connection_scope,
                    runtime,
                    responder,
                    service,
                    error_handler,
                ));
            }

            Ok(None) => {
                debug!(target: "overseerd::daemon", "connection closed by peer");
                break;
            }

            Err(e) => {
                warn!(target: "overseerd::daemon", error = %e, "connection error");
                break;
            }
        }
    }

    // The connection (and its call table) is dropped here, cancelling in-flight
    // calls via their tokens; abort any handler tasks still winding down.
    tasks.abort_all();

    debug!(target: "overseerd::daemon", "connection ended");
}

/// Drives one call to completion on its own task: build its request scope, dispatch,
/// then pump the outcome into the matching responder — a single reply for unary calls,
/// or a stream of items terminated by `finish`/`error` for streaming calls.
#[allow(clippy::too_many_arguments)]
async fn drive_call<R>(
    path: String,
    payload: Vec<u8>,
    requests: Option<mpsc::Receiver<Vec<u8>>>,
    cancel: CancellationToken,
    peer: PeerInfo,
    connection_scope: Arc<ScopeContainer>,
    runtime: AppRuntime,
    responder: R,
    mut service: RpcService,
    error_handler: Option<Arc<dyn ErrorHandler>>,
) where
    R: Respond + RespondStream + Send + 'static,
{
    let request_scope = match runtime
        .open_scope(&RequestScope, connection_scope, Vec::new())
        .await
    {
        Ok(scope) => scope,

        Err(e) => {
            error!(target: "overseerd::daemon", %path, error = %e, "request scope build failed");
            let response = apply_error_handler(&error_handler, &path, ErrorResponse::from(e)).await;
            let _ = responder
                .respond(CallResult::Err {
                    code: response.code,
                    body: response.body,
                })
                .await;

            return;
        }
    };

    let ctx = RpcCallContext::new(payload, peer, request_scope, requests, cancel);
    let request = RpcRequest::new(path.clone(), ctx);

    // Drive the request through the middleware stack; its terminal service is the
    // router. `ready` honours the tower contract for layers that exert backpressure.
    let outcome = match service.ready().await {
        Ok(svc) => svc.call(request).await,

        Err(e) => Err(e),
    };

    match outcome {
        Ok(RpcOutcome::Unary(RpcResponse { payload })) => {
            debug!(target: "overseerd::daemon", %path, "call succeeded");

            if let Err(e) = responder.respond(CallResult::Ok(payload)).await {
                warn!(target: "overseerd::daemon", %path, error = %e, "failed to send response");
            }
        }

        Ok(RpcOutcome::Stream(mut stream)) => {
            debug!(target: "overseerd::daemon", %path, "streaming response");

            let mut sink = responder.into_sink();

            loop {
                match stream.next().await {
                    Some(Ok(item)) => {
                        if let Err(e) = sink.send(item).await {
                            warn!(target: "overseerd::daemon", %path, error = %e, "failed to send stream item");

                            return;
                        }
                    }

                    Some(Err(e)) => {
                        warn!(target: "overseerd::daemon", %path, code = ?e.code, "stream handler errored");
                        let e = apply_error_handler(&error_handler, &path, e).await;
                        let _ = sink.error(e.code, e.body).await;

                        return;
                    }

                    None => break,
                }
            }

            if let Err(e) = sink.finish().await {
                warn!(target: "overseerd::daemon", %path, error = %e, "failed to finish stream");
            }
        }

        Err(e) => {
            warn!(target: "overseerd::daemon", %path, code = ?e.code, "call returned error");
            let e = apply_error_handler(&error_handler, &path, e).await;

            if let Err(e) = responder
                .respond(CallResult::Err {
                    code: e.code,
                    body: e.body,
                })
                .await
            {
                warn!(target: "overseerd::daemon", %path, error = %e, "failed to send error response");
            }
        }
    }
}

/// Applies the global [`ErrorHandler`] to an outgoing error, or passes it through
/// unchanged when none is registered.
async fn apply_error_handler(
    handler: &Option<Arc<dyn ErrorHandler>>,
    path: &str,
    error: ErrorResponse,
) -> ErrorResponse {
    match handler {
        Some(handler) => handler.handle(path, error).await,

        None => error,
    }
}
