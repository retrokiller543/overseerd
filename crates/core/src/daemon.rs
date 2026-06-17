use std::{fmt, sync::Arc};

use overseer_transport::{CallResult, Connection, Respond, Transport};
use tracing::{debug, error, info, instrument, trace, warn};

use crate::{
    ServiceComponent,
    connection::{ConnectionHandler, ConnectionInfo},
    container::ComponentContainer,
    descriptors::{
        BoxedComponent, Component, ComponentDescriptor, ComponentScope, RpcCallContext, RpcGroup,
        RpcResponse, ServiceDescriptor, TypeDescriptor,
    },
    lifecycle::{ShutdownHandle, ShutdownSignal},
    registry::DescriptorRegistry,
    router::RpcRouter,
};

/// Assembles a Daemon from an explicit set of components and services.
pub struct DaemonBuilder {
    name: String,
    registry: DescriptorRegistry,
    connection_handlers: Vec<Box<dyn ConnectionHandler>>,
    instances: Vec<BoxedComponent>,
}

impl DaemonBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registry: DescriptorRegistry::default(),
            connection_handlers: Vec::new(),
            instances: Vec::new(),
        }
    }

    /// Registers a pre-built singleton instance (e.g. a stateful service
    /// constructed by hand). Synthesizes a `factory: None` descriptor in the
    /// registry (so the declaration is visible and dependencies validate) and
    /// holds the instance until the container is built. The type's identity
    /// comes from its `Component` impl (`#[derive(Component)]` / `#[service]`).
    pub fn with_component<T: Component>(mut self, value: T) -> Self {
        self.registry
            .components
            .push(Self::generate_component_descriptor::<T>());

        self.instances.push(BoxedComponent {
            ty: TypeDescriptor::of::<T>(T::NAME),
            value: Box::new(Arc::new(value)),
        });

        self
    }

    const fn generate_component_descriptor<T: Component>() -> ComponentDescriptor {
        ComponentDescriptor {
            id: T::ID,
            name: T::NAME,
            ty: TypeDescriptor::of::<T>(T::NAME),
            scope: ComponentScope::Singleton,
            dependencies: &[],
            factory: None,
            default_factory: false,
        }
    }

    /// Merges all `inventory::submit!` entries in the binary into the registry,
    /// preserving anything already registered (manual components, instances).
    pub fn auto_discover(mut self) -> Self {
        let discovered = DescriptorRegistry::collect();

        self.registry.components.extend(discovered.components);
        self.registry.services.extend(discovered.services);
        self.registry.rpc_groups.extend(discovered.rpc_groups);

        self
    }

    /// Registers a pre-built service singleton and its header.
    ///
    /// Synthesizes the [`ServiceDescriptor`] from the type's `ServiceComponent`
    /// impl (id, name, version) and registers the instance like
    /// [`with_component`](Self::with_component). Pair with `#[handlers]` +
    /// [`auto_discover`](Self::auto_discover) or explicit [`rpcs`](Self::rpcs)
    /// to supply the methods; do not also auto-discover the same service or its
    /// header is registered twice.
    pub fn with_service<T: ServiceComponent>(mut self, value: T) -> Self {
        self.registry.services.push(ServiceDescriptor {
            id: T::ID,
            name: T::NAME,
            ty: TypeDescriptor::of::<T>(T::NAME),
            version: T::VERSION,
        });

        self.with_component(value)
    }

    /// Manually register a component descriptor for construction during build.
    /// Prefer [`with_component`](Self::with_component) for instances, or the
    /// [`component`](overseer_macros::component) macro to generate the descriptor.
    pub fn component(mut self, descriptor: &'static ComponentDescriptor) -> Self {
        self.registry.components.push(*descriptor);

        self
    }

    /// Manually register a service header (prefer the [`service`](overseer_macros::service) macro).
    pub fn service(mut self, descriptor: &'static ServiceDescriptor) -> Self {
        self.registry.services.push(*descriptor);

        self
    }

    /// Registers a group of RPCs contributed to the service of a matching type.
    pub fn rpcs(mut self, group: &'static RpcGroup) -> Self {
        self.registry.rpc_groups.push(*group);

        self
    }

    /// Registers a handler called once per accepted connection to populate
    /// connection-scoped context. Handlers run in registration order.
    pub fn connection_handler<H: ConnectionHandler>(mut self, handler: H) -> Self {
        self.connection_handlers.push(Box::new(handler));

        self
    }

    /// Validates the registry, resolves all components, and builds a ready-to-run Daemon.
    pub async fn build(self) -> crate::Result<Daemon> {
        debug!(daemon = %self.name, "building daemon");

        let mut registry = self.registry;

        registry.validate()?;

        // Collapse to the effective component set (explicit factories override
        // field-injection defaults) so the stored registry reflects what runs.
        let resolved = registry.resolved_components()?;
        registry.components = resolved;

        let container = ComponentContainer::build(&registry.components, self.instances).await?;
        let router = RpcRouter::from_registry(&registry);
        let shutdown = ShutdownSignal::new();

        info!(
            daemon = %self.name,
            components = registry.components.len(),
            services = registry.services.len(),
            "daemon built"
        );

        Ok(Daemon {
            name: self.name,
            registry,
            container,
            router,
            shutdown,
            connection_handlers: self.connection_handlers,
        })
    }
}

/// A fully assembled daemon, ready to accept connections and dispatch RPC calls.
pub struct Daemon {
    pub name: String,
    pub registry: DescriptorRegistry,
    pub container: ComponentContainer,
    pub router: RpcRouter,
    shutdown: ShutdownSignal,
    connection_handlers: Vec<Box<dyn ConnectionHandler>>,
}

impl fmt::Debug for Daemon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Daemon")
            .field("name", &self.name)
            .field("components", &self.registry.components.len())
            .field("services", &self.registry.services.len())
            .field("routes", &self.router.route_count())
            .field("connection_handlers", &self.connection_handlers.len())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for Daemon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Daemon: {}", self.name)?;
        write!(f, "{}", self.registry)?;

        Ok(())
    }
}

impl Daemon {
    pub fn builder(name: impl Into<String>) -> DaemonBuilder {
        DaemonBuilder::new(name)
    }

    /// Returns a handle that can trigger graceful shutdown from any spawned task.
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        self.shutdown.handle()
    }

    /// Serves RPC calls from `transport` until ctrl-c or a shutdown signal.
    ///
    /// One task is spawned per accepted connection. Within each connection task,
    /// calls are dispatched sequentially — the response is sent before the next
    /// call is read.
    pub async fn serve<T>(self, mut transport: T) -> crate::Result<()>
    where
        T: Transport,
        T::Connection: 'static,
    {
        let transport_name = std::any::type_name::<T>();

        info!(daemon = %self.name, transport = transport_name, "serve starting");

        let router = Arc::new(self.router);
        let handlers = Arc::new(self.connection_handlers);
        let components = Arc::new(self.container);
        let mut shutdown = self.shutdown;

        loop {
            tokio::select! {
                result = transport.accept() => {
                    match result {
                        Ok(conn) => {
                            debug!(peer = ?conn.peer().addr, "connection accepted, spawning task");

                            let router = Arc::clone(&router);
                            let handlers = Arc::clone(&handlers);
                            let components = Arc::clone(&components);

                            tokio::spawn(async move {
                                serve_connection(conn, router, handlers, components).await;
                            });
                        }

                        Err(e) => {
                            error!(error = %e, "transport accept failed");
                            return Err(e.into());
                        }
                    }
                }

                _ = tokio::signal::ctrl_c() => {
                    info!("ctrl-c received, shutting down");
                    break;
                }

                _ = shutdown.wait() => {
                    info!("shutdown signal received");
                    break;
                }
            }
        }

        info!(transport = transport_name, "serve stopped");

        Ok(())
    }

    /// Waits for ctrl-c or a shutdown signal without serving any transport.
    pub async fn run(self) -> crate::Result<()> {
        let mut shutdown = self.shutdown;

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = shutdown.wait() => {},
        }

        Ok(())
    }
}

#[instrument(
    skip_all,
    fields(peer = ?conn.peer().addr),
    name = "connection"
)]
async fn serve_connection<C: Connection>(
    mut conn: C,
    router: Arc<RpcRouter>,
    handlers: Arc<Vec<Box<dyn ConnectionHandler>>>,
    components: Arc<ComponentContainer>,
) {
    debug!("connection established");

    let mut info = ConnectionInfo::new(conn.peer().clone());

    for (i, handler) in handlers.iter().enumerate() {
        trace!(index = i, "running connection handler");

        if let Err(e) = handler.on_connect(&mut info).await {
            error!(error = %e, "connection handler failed, closing");
            return;
        }
    }

    let info = Arc::new(info);

    debug!("connection ready");

    loop {
        match conn.recv().await {
            Ok(Some((call, responder))) => {
                let path = call.path.clone();

                debug!(%path, "dispatching call");

                let ctx = RpcCallContext {
                    payload: call.payload,
                    connection: Arc::clone(&info),
                    components: Arc::clone(&components),
                };

                let outcome = match router.dispatch(&path, ctx).await {
                    Ok(RpcResponse { payload }) => {
                        debug!(%path, "call succeeded");
                        CallResult::Ok(payload)
                    }

                    Err(e) => {
                        warn!(%path, error = %e, "call returned error");
                        CallResult::Err(e.to_string())
                    }
                };

                if let Err(e) = responder.respond(outcome).await {
                    warn!(%path, error = %e, "failed to send response");
                    break;
                }
            }

            Ok(None) => {
                debug!("connection closed by peer");
                break;
            }

            Err(e) => {
                warn!(error = %e, "connection error");
                break;
            }
        }
    }

    debug!("connection ended");
}
