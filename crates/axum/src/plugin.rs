//! The axum protocol plugin and its builder extension.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::routing::Route;
use overseerd_app::{AppBuilder, AppRegistry, AppRuntime, Plugin, ProtocolPlugin};
use overseerd_core::{Descriptor, Scope, TypeDescriptor};
use overseerd_di::{BoxedComponent, Component, ComponentDescriptor};
use tower::{Layer, Service};

use crate::controller::{CONTROLLERS, ControllerDescriptor};
use crate::extract::ScopeHandle;
use crate::middleware::{AxumMiddleware, MiddlewareApplier, as_layer};
use crate::protocol::Axum;
use crate::request_meta::{REQUEST_META_DESCRIPTOR, RequestMeta};
use crate::scope::{Connection as ConnectionScope, Request as RequestScope};

/// The axum HTTP protocol plugin.
///
/// Accumulates the registered/discovered controllers, contributes no extra DI seeds, and
/// builds the [`Axum`] protocol: each controller's [`axum::Router`] merged together and
/// wrapped by a per-request scope layer that opens the request scope and threads it into
/// the request extensions for the [`Inject`](crate::Inject) extractor.
#[derive(Default)]
pub struct AxumPlugin {
    controllers: Vec<ControllerDescriptor>,

    /// Global middleware, in registration order — both raw `tower::Layer`s (via
    /// [`AxumAppBuilder::layer`]) and DI-resolved [`AxumMiddleware`]s (via
    /// [`AxumAppBuilder::middleware`]) accumulate here, so ordering between the two is just
    /// registration order.
    middleware: Vec<MiddlewareApplier>,

    /// Discovered `#[controller(ws = ..)]` descriptors. Only mounted for protocols a user opts into
    /// via [`register_ws`](AxumAppBuilder::register_ws).
    #[cfg(feature = "ws")]
    ws_controllers: Vec<crate::ws::WsControllerDescriptor>,

    /// Opt-in ws endpoints: each pairs a protocol type with the path to mount its upgrade handler.
    #[cfg(feature = "ws")]
    ws_registrations: Vec<WsRegistration>,
}

/// A mount closure: builds one ws endpoint's router + handle from its controllers and runtime. A
/// boxed `FnOnce` (not a `fn` pointer) so it can capture the path and the protocol's `Options`,
/// whose type varies by protocol.
#[cfg(feature = "ws")]
type WsMount = Box<
    dyn FnOnce(
        Vec<crate::ws::WsControllerDescriptor>,
        &AppRuntime,
    ) -> (axum::Router, crate::ws::WebsocketHandler),
>;

/// One opt-in ws endpoint: a protocol type (by [`TypeId`](std::any::TypeId)) bound to a path, with a
/// [`WsMount`] closure that already captures the path and the protocol's `Options`.
#[cfg(feature = "ws")]
struct WsRegistration {
    path: String,
    protocol: std::any::TypeId,
    mount: WsMount,
}

impl Plugin for AxumPlugin {
    fn auto_discover(&mut self) {
        self.controllers.extend(CONTROLLERS.iter().copied());

        #[cfg(feature = "ws")]
        self.ws_controllers
            .extend(crate::ws::WS_CONTROLLERS.iter().copied());
    }

    fn register(&self, registry: &mut AppRegistry) {
        registry.components.push(REQUEST_META_DESCRIPTOR);
    }
}

impl ProtocolPlugin for AxumPlugin {
    type Protocol = Axum;
    type Error = crate::Error;

    // Root→leaf: `Connection` (WebSocket-only) outlives `Request`. A plain HTTP request opens only
    // `Request` (parented at root); a ws message opens `Request` parented at its `Connection`.
    const SCOPES: &'static [&'static dyn Scope] = &[&ConnectionScope, &RequestScope];

    fn build(self, runtime: &AppRuntime) -> crate::Result<Axum> {
        // Merge every controller's routes. Each builder resolves its controller singleton
        // from the runtime and captures it in the route handlers, so the merged router owns
        // ready-to-call handlers.
        let mut router = axum::Router::new();

        for descriptor in &self.controllers {
            router = router.merge((descriptor.router)(runtime));
        }

        // WebSocket endpoints are opt-in: for each `register_ws::<P>(path)`, select the ws
        // controllers that speak `P`, let `P::build` set up its own routing, and merge the
        // path-scoped router. The protocol owns dispatch; we only mount it and keep its handle.
        #[cfg(feature = "ws")]
        let ws_endpoints = {
            let mut endpoints = Vec::with_capacity(self.ws_registrations.len());

            for registration in self.ws_registrations {
                let controllers: Vec<crate::ws::WsControllerDescriptor> = self
                    .ws_controllers
                    .iter()
                    .copied()
                    .filter(|descriptor| (descriptor.protocol)() == registration.protocol)
                    .collect();

                // The mount closure already holds the path and the protocol's options.
                let (ws_router, handler) = (registration.mount)(controllers, runtime);

                router = router.merge(ws_router);
                endpoints.push(handler);
            }

            endpoints
        };

        // Global middleware, first-registered outermost (mirroring the RPC protocol's own
        // convention): `axum::Router::layer` stacks last-applied-outermost, so folding in
        // reverse registration order makes the first-registered applier the outermost one.
        // These must stay *inside* (applied before) the scope-open layer below so every piece
        // of user middleware — raw or DI-backed — can `Inject` request-scoped state.
        for applier in self.middleware.into_iter().rev() {
            router = applier(runtime, router);
        }

        // The bridge: a per-request layer that opens the Request scope (parented at the
        // singleton root) and inserts its handle into the request extensions. `Inject`
        // reads it back out; a scope-build failure degrades to 500 rather than panicking.
        // Also seeds `RequestMeta` (method/URI/headers/cookies) so request-scoped components
        // and handlers can depend on the native request without axum's own extractors
        // entering the DI graph.
        let scope_runtime = runtime.clone();
        let router = router.layer(middleware::from_fn(
            move |mut request: Request, next: Next| {
                let scope_runtime = scope_runtime.clone();

                async move {
                    let parent = Arc::clone(scope_runtime.root());

                    let meta = RequestMeta::from_parts(
                        request.method().clone(),
                        request.uri().clone(),
                        request.headers().clone(),
                    );
                    let seed = BoxedComponent {
                        ty: TypeDescriptor::of::<RequestMeta>("RequestMeta"),
                        value: Box::new(meta),
                    };

                    match scope_runtime
                        .open_scope(&RequestScope, parent, vec![seed])
                        .await
                    {
                        Ok(scope) => {
                            request.extensions_mut().insert(ScopeHandle(scope));

                            next.run(request).await
                        }

                        Err(error) => {
                            tracing::error!(
                                target: "overseerd::axum",
                                error = %error,
                                "request scope build failed"
                            );

                            axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
                        }
                    }
                }
            },
        ));

        let axum = Axum::new(router);

        #[cfg(feature = "ws")]
        let axum = axum.with_ws_endpoints(ws_endpoints);

        Ok(axum)
    }
}

/// axum-specific builder methods, contributed to [`AppBuilder<AxumPlugin>`] as an extension
/// trait. Bring it into scope to register controllers by type; it is in the prelude.
///
/// Controllers also auto-register through the [`CONTROLLERS`] slice, so an
/// `auto_discover`d app needs no explicit registration — this is the explicit path (and
/// the one the `app!` macro could drive).
pub trait AxumAppBuilder {
    /// Registers controller type `T`: its route header and its construction factory.
    fn controller<T>(self) -> Self
    where
        T: Descriptor<ControllerDescriptor> + Descriptor<ComponentDescriptor>;

    /// Manually registers a raw controller header (prefer [`controller`](Self::controller)).
    fn controller_descriptor(self, descriptor: &'static ControllerDescriptor) -> Self;

    /// Wraps the whole app in a raw `tower::Layer` — any standard axum/tower middleware
    /// (`tower-http`, a hand-written `axum::middleware::from_fn`, …) works here unmodified,
    /// with the same bound as [`axum::Router::layer`] itself. The first layer registered is
    /// the outermost; interleaves in registration order with [`middleware`](Self::middleware).
    fn layer<L>(self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static;

    /// Sugar over [`layer`](Self::layer): resolves `M` as a DI singleton (shared across every
    /// attach point it's registered at, instead of constructed per attach point) and wraps it
    /// via [`as_layer`].
    fn middleware<M>(self) -> Self
    where
        M: AxumMiddleware + Component<Handle = Arc<M>> + Descriptor<ComponentDescriptor>;

    /// Opts the app into a WebSocket protocol `P`, mounting its upgrade handler at `path` with the
    /// protocol's default [`Options`](crate::ws::WebsocketProtocol::Options). Only
    /// `#[controller(ws = P)]` controllers speaking `P` are then served, under `path` (the path
    /// can't be inferred, so it is given here). Call it once per protocol to run, e.g., a STOMP
    /// endpoint and a `JsonWs` endpoint on different paths in one server. Rejects two protocols on
    /// the same path at build.
    #[cfg(feature = "ws")]
    fn register_ws<P>(self, path: impl Into<String>) -> Self
    where
        P: crate::ws::WebsocketProtocol,
        P::Options: Default;

    /// Like [`register_ws`](Self::register_ws), but with explicit per-endpoint
    /// [`Options`](crate::ws::WebsocketProtocol::Options) — e.g. a `StompConfig` selecting the STOMP
    /// heart-beat interval and accepted versions.
    #[cfg(feature = "ws")]
    fn register_ws_with<P>(self, path: impl Into<String>, options: P::Options) -> Self
    where
        P: crate::ws::WebsocketProtocol;
}

impl AxumAppBuilder for AppBuilder<AxumPlugin> {
    fn controller<T>(mut self) -> Self
    where
        T: Descriptor<ControllerDescriptor> + Descriptor<ComponentDescriptor>,
    {
        self.protocol_mut()
            .controllers
            .push(<T as Descriptor<ControllerDescriptor>>::DESCRIPTOR);

        self.component::<T>()
    }

    fn controller_descriptor(mut self, descriptor: &'static ControllerDescriptor) -> Self {
        self.protocol_mut().controllers.push(*descriptor);

        self
    }

    fn layer<L>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request>>::Future: Send + 'static,
    {
        self.protocol_mut()
            .middleware
            .push(Box::new(move |_runtime, router| router.layer(layer)));

        self
    }

    fn middleware<M>(mut self) -> Self
    where
        M: AxumMiddleware + Component<Handle = Arc<M>> + Descriptor<ComponentDescriptor>,
    {
        self.protocol_mut()
            .middleware
            .push(Box::new(|runtime, router| {
                let mw = runtime
                    .root()
                    .get::<M>()
                    .expect("middleware component missing from DI root — did you register it?");

                router.layer(as_layer(mw))
            }));

        self.component::<M>()
    }

    #[cfg(feature = "ws")]
    fn register_ws<P>(self, path: impl Into<String>) -> Self
    where
        P: crate::ws::WebsocketProtocol,
        P::Options: Default,
    {
        self.register_ws_with::<P>(path, P::Options::default())
    }

    #[cfg(feature = "ws")]
    fn register_ws_with<P>(mut self, path: impl Into<String>, options: P::Options) -> Self
    where
        P: crate::ws::WebsocketProtocol,
    {
        let path = path.into();

        let duplicate = self
            .protocol_mut()
            .ws_registrations
            .iter()
            .any(|registration| registration.path == path);

        assert!(
            !duplicate,
            "register_ws: a websocket protocol is already mounted at `{path}`"
        );

        // Capture the path and options in the mount closure so the non-generic registration can
        // carry protocol-specific `Options` without erasing their type.
        let mount_path = path.clone();
        let mount = Box::new(move |controllers, runtime: &AppRuntime| {
            crate::ws::mount_ws::<P>(&mount_path, controllers, runtime, options)
        });

        self.protocol_mut().ws_registrations.push(WsRegistration {
            path,
            protocol: std::any::TypeId::of::<P>(),
            mount,
        });

        self
    }
}
