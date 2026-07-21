//! The native RPC protocol plugin and its builder extension.

use std::sync::Arc;

use overseerd_app::{AppBuilder, AppRegistry, AppRuntime, Plugin, ProtocolPlugin};
use overseerd_core::{Descriptor, Scope, TypeDescriptor};
use overseerd_di::{ComponentDescriptor, ServiceComponent};
use overseerd_transport::PeerInfo;
use tower::{Layer, Service};

use crate::descriptors::{RpcOutcome, SERVICES, ServiceDescriptor};
use crate::extract::ErrorResponse;
use crate::middleware::{ErrorHandler, Guard, GuardLayer, RouterService, RpcRequest, RpcService};
use crate::protocol::{Rpc, RpcLimits};
use crate::router::RpcRouter;
use crate::scope::{Connection as ConnectionScope, Request as RequestScope};

/// A registered middleware step: wraps the current dispatch service in one more layer.
/// Collected in registration order and applied outermost-first when the app is built.
type LayerApplier = Box<dyn FnOnce(RpcService) -> RpcService + Send>;

/// The framework-provided connection-scoped injectable for the remote peer.
///
/// Seeded into every connection scope with the actual `PeerInfo`, so a connection-scoped
/// component can depend on `Arc<PeerInfo>` (e.g. to authenticate in its constructor).
static PEER_INFO_DESCRIPTOR: ComponentDescriptor = ComponentDescriptor::manual(
    "__overseerd_peer_info",
    "PeerInfo",
    TypeDescriptor::of::<PeerInfo>("PeerInfo"),
    &ConnectionScope,
);

/// The native RPC protocol plugin.
///
/// Accumulates the RPC-specific builder state — the discovered/registered services, the
/// middleware layers, and the global error handler — seeds the connection-scoped
/// `PeerInfo` via [`Plugin::register`], and builds the [`Rpc`] protocol (the router
/// wrapped by the middleware stack) via [`ProtocolPlugin::build`].
#[derive(Default)]
pub struct RpcPlugin {
    services: Vec<ServiceDescriptor>,
    layers: Vec<LayerApplier>,
    error_handler: Option<Arc<dyn ErrorHandler>>,
    limits: RpcLimits,
}

impl Plugin for RpcPlugin {
    fn auto_discover(&mut self) {
        self.services.extend(SERVICES.iter().copied());
    }

    fn register(&self, registry: &mut AppRegistry) {
        registry.components.push(PEER_INFO_DESCRIPTOR);
    }
}

impl ProtocolPlugin for RpcPlugin {
    type Protocol = Rpc;
    type Error = crate::Error;

    const SCOPES: &'static [&'static dyn Scope] = &[&ConnectionScope, &RequestScope];

    fn build(self, runtime: &AppRuntime) -> crate::Result<Rpc> {
        let resolved = crate::routes::resolved_services(&self.services);
        crate::routes::validate_services(&resolved)?;

        // Does any real component depend on the peer? If not, the connection scope need
        // not exist solely to hold it; handlers still reach the peer via the `Peer`
        // extractor, which reads it off the call context rather than the scope chain.
        let peer_id = PEER_INFO_DESCRIPTOR.ty.type_id;
        let needs_peer = runtime.resolved_components().iter().any(|c| {
            c.ty.type_id != peer_id && c.dependencies().iter().any(|d| d.ty.type_id == peer_id)
        });

        let router = Arc::new(RpcRouter::from_services(&resolved));

        // Fold the registered layers onto the terminal router service. Appliers are
        // pushed in registration order, so applying them in reverse makes the
        // first-registered layer the outermost wrapper.
        let mut service: RpcService = RpcService::new(RouterService::new(Arc::clone(&router)));

        for applier in self.layers.into_iter().rev() {
            service = applier(service);
        }

        Ok(Rpc::new(
            router,
            service,
            self.error_handler,
            needs_peer,
            self.limits,
        ))
    }
}

/// RPC-specific builder methods, contributed to [`AppBuilder<RpcPlugin>`] as an extension
/// trait (a foreign crate cannot add inherent methods to a generic type). Bring it into
/// scope to register services, middleware, guards, and the error handler; it is in the
/// prelude.
pub trait RpcAppBuilder {
    /// Registers service type `T` by type: its identity header (carrying its RPC surface)
    /// and its construction factory.
    fn service<T>(self) -> Self
    where
        T: Descriptor<ServiceDescriptor> + Descriptor<ComponentDescriptor>;

    /// Registers a pre-built service singleton: its identity header and the instance.
    fn with_service<T>(self, value: T) -> Self
    where
        T: ServiceComponent + Descriptor<ServiceDescriptor>;

    /// Manually registers a raw service header (prefer [`service`](Self::service) by type).
    fn service_descriptor(self, descriptor: &'static ServiceDescriptor) -> Self;

    /// Wraps the dispatch path in a [`tower::Layer`], running on every call. The first
    /// layer registered is the outermost.
    fn middleware<L>(self, layer: L) -> Self
    where
        L: Layer<RpcService> + Send + 'static,
        L::Service: Service<RpcRequest, Response = RpcOutcome, Error = ErrorResponse>
            + Clone
            + Send
            + 'static,
        <L::Service as Service<RpcRequest>>::Future: Send + 'static;

    /// Registers a [`Guard`] as a pre-handler admit/reject check.
    fn guard<G: Guard>(self, guard: G) -> Self;

    /// Sets the single global [`ErrorHandler`] applied to every error response.
    fn error_handler<H: ErrorHandler>(self, handler: H) -> Self;

    /// Sets connection and per-connection call admission limits.
    fn rpc_limits(self, limits: RpcLimits) -> Self;
}

impl RpcAppBuilder for AppBuilder<RpcPlugin> {
    fn service<T>(mut self) -> Self
    where
        T: Descriptor<ServiceDescriptor> + Descriptor<ComponentDescriptor>,
    {
        self.protocol_mut()
            .services
            .push(<T as Descriptor<ServiceDescriptor>>::DESCRIPTOR);

        self.component::<T>()
    }

    fn with_service<T>(mut self, value: T) -> Self
    where
        T: ServiceComponent + Descriptor<ServiceDescriptor>,
    {
        self.protocol_mut()
            .services
            .push(<T as Descriptor<ServiceDescriptor>>::DESCRIPTOR);

        self.with_component(value)
    }

    fn service_descriptor(mut self, descriptor: &'static ServiceDescriptor) -> Self {
        self.protocol_mut().services.push(*descriptor);

        self
    }

    fn middleware<L>(mut self, layer: L) -> Self
    where
        L: Layer<RpcService> + Send + 'static,
        L::Service: Service<RpcRequest, Response = RpcOutcome, Error = ErrorResponse>
            + Clone
            + Send
            + 'static,
        <L::Service as Service<RpcRequest>>::Future: Send + 'static,
    {
        self.protocol_mut()
            .layers
            .push(Box::new(move |inner| RpcService::new(layer.layer(inner))));

        self
    }

    fn guard<G: Guard>(self, guard: G) -> Self {
        self.middleware(GuardLayer::new(Arc::new(guard)))
    }

    fn error_handler<H: ErrorHandler>(mut self, handler: H) -> Self {
        self.protocol_mut().error_handler = Some(Arc::new(handler));

        self
    }

    fn rpc_limits(mut self, limits: RpcLimits) -> Self {
        self.protocol_mut().limits = limits;

        self
    }
}
