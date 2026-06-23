use std::{
    any::TypeId,
    collections::HashSet,
    fmt,
    sync::Arc,
};

use futures::StreamExt;
use overseer_transport::{
    CallResult, Connection, PeerInfo, Respond, RespondStream, ResponseSink, Transport,
};
use tokio::{sync::mpsc, task::JoinSet};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};

use crate::{
    ServiceComponent,
    container::{ScopeContainer, ScopeRegistry, topological_sort},
    descriptors::{
        BoxedComponent, Component, ComponentDescriptor, ComponentScope, RpcCallContext, RpcGroup,
        RpcOutcome, RpcResponse, ServiceDescriptor, TypeDescriptor,
    },
    extract::ErrorResponse,
    lifecycle::{ShutdownHandle, ShutdownSignal},
    registry::DescriptorRegistry,
    router::RpcRouter,
};

/// The framework-provided connection-scoped injectable for the remote peer.
///
/// Seeded into every connection scope with the actual `PeerInfo`, so a
/// connection-scoped component can depend on `Arc<PeerInfo>` (e.g. to authenticate
/// in its constructor) — the DI-native replacement for the old `on_connect` hook.
static PEER_INFO_DESCRIPTOR: ComponentDescriptor = ComponentDescriptor {
    id: "__overseer_peer_info",
    name: "PeerInfo",
    ty: TypeDescriptor::of::<PeerInfo>("PeerInfo"),
    scope: ComponentScope::Connection,
    dependencies: &[],
    factory: None,
    default_factory: false,
};

/// Assembles a Daemon from an explicit set of components and services.
pub struct DaemonBuilder {
    name: String,
    registry: DescriptorRegistry,
    instances: Vec<BoxedComponent>,
}

impl DaemonBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registry: DescriptorRegistry::default(),
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
            .push(ComponentDescriptor::of::<T>());

        self.instances.push(BoxedComponent {
            ty: TypeDescriptor::of::<T>(T::NAME),
            value: Box::new(value.into_handle()),
        });

        self
    }

    /// Merges every link-time-registered descriptor in the binary into the
    /// registry, preserving anything already registered (manual components,
    /// instances).
    pub fn auto_discover(mut self) -> Self {
        let discovered = DescriptorRegistry::collect();

        self.registry.components.extend(discovered.components);
        self.registry.services.extend(discovered.services);
        self.registry.rpc_groups.extend(discovered.rpc_groups);
        self.registry.providers.extend(discovered.providers);

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
        self.registry.services.push(ServiceDescriptor::of::<T>());
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

    /// Validates the registry, resolves all components, partitions them by scope,
    /// and builds a ready-to-run Daemon.
    pub async fn build(self) -> crate::Result<Daemon> {
        debug!(daemon = %self.name, "building daemon");

        let mut registry = self.registry;

        // The peer is a framework-provided connection-scoped injectable; declare it
        // so dependencies on it validate and it partitions into the connection scope.
        registry.components.push(PEER_INFO_DESCRIPTOR);

        registry.validate()?;

        // Collapse to the effective component set (explicit factories override
        // field-injection defaults) so the stored registry reflects what runs.
        let resolved = registry.resolved_components()?;
        registry.components = resolved.clone();

        let scopes = ScopePlan::partition(&resolved, &registry.providers)?;

        let scope_registry = Arc::new(ScopeRegistry::new(
            scopes.transient,
            registry.providers.clone(),
        ));
        let root =
            ScopeContainer::build_root(&scopes.singletons, self.instances, Arc::clone(&scope_registry))
                .await?;
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
            root,
            scopes: scope_registry,
            connection_order: Arc::new(scopes.connection_order),
            request_order: Arc::new(scopes.request_order),
            router,
            shutdown,
        })
    }
}

/// The per-scope construction plan computed once at daemon build.
struct ScopePlan {
    singletons: Vec<ComponentDescriptor>,
    connection_order: Vec<ComponentDescriptor>,
    request_order: Vec<ComponentDescriptor>,
    transient: std::collections::HashMap<TypeId, ComponentDescriptor>,
}

impl ScopePlan {
    /// Splits the resolved components by scope and precomputes the construction
    /// order for the connection and request scopes (singletons are sorted by
    /// `build_root`; transients are constructed on demand, so need no order).
    fn partition(
        resolved: &[ComponentDescriptor],
        providers: &[crate::descriptors::ProviderDescriptor],
    ) -> crate::Result<Self> {
        let mut singletons = Vec::new();
        let mut connection_components = Vec::new();
        let mut request_components = Vec::new();
        let mut transient = std::collections::HashMap::new();

        for c in resolved {
            match c.scope {
                ComponentScope::Singleton => singletons.push(*c),
                // A connection-scoped manual instance (factory None) — only the
                // framework's PeerInfo — is seeded per connection, not constructed.
                ComponentScope::Connection if c.factory.is_some() => {
                    connection_components.push(*c)
                }
                ComponentScope::Connection => {}
                ComponentScope::Request if c.factory.is_some() => request_components.push(*c),
                ComponentScope::Request => {}
                ComponentScope::Transient => {
                    transient.insert((c.ty.type_id)(), *c);
                }
            }
        }

        // Connection components resolve against singletons and the seeded peer.
        let peer_id = (PEER_INFO_DESCRIPTOR.ty.type_id)();
        let mut conn_prebuilt: HashSet<TypeId> =
            singletons.iter().map(|c| (c.ty.type_id)()).collect();
        conn_prebuilt.insert(peer_id);

        let connection_order = topological_sort(&connection_components, &conn_prebuilt, providers)?
            .into_iter()
            .copied()
            .collect();

        // Request components resolve against singletons, the peer, and all
        // connection-scoped components.
        let mut req_prebuilt = conn_prebuilt.clone();
        req_prebuilt.extend(connection_components.iter().map(|c| (c.ty.type_id)()));

        let request_order = topological_sort(&request_components, &req_prebuilt, providers)?
            .into_iter()
            .copied()
            .collect();

        Ok(Self {
            singletons,
            connection_order,
            request_order,
            transient,
        })
    }
}

/// A fully assembled daemon, ready to accept connections and dispatch RPC calls.
pub struct Daemon {
    pub name: String,
    pub registry: DescriptorRegistry,
    root: Arc<ScopeContainer>,
    scopes: Arc<ScopeRegistry>,
    connection_order: Arc<Vec<ComponentDescriptor>>,
    request_order: Arc<Vec<ComponentDescriptor>>,
    router: RpcRouter,
    shutdown: ShutdownSignal,
}

impl fmt::Debug for Daemon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Daemon")
            .field("name", &self.name)
            .field("components", &self.registry.components.len())
            .field("services", &self.registry.services.len())
            .field("routes", &self.router.route_count())
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

    /// The root (singleton) scope container.
    pub fn container(&self) -> &Arc<ScopeContainer> {
        &self.root
    }

    /// Returns a handle that can trigger graceful shutdown from any spawned task.
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        self.shutdown.handle()
    }

    /// Serves RPC calls from `transport` until ctrl-c or a shutdown signal.
    ///
    /// One task is spawned per accepted connection, and within a connection each
    /// call is driven on its own task so streaming calls run concurrently and
    /// the connection keeps reading inbound stream frames while handlers run.
    pub async fn serve<T>(self, mut transport: T) -> crate::Result<()>
    where
        T: Transport,
        T::Connection: 'static,
    {
        let transport_name = std::any::type_name::<T>();

        info!(daemon = %self.name, transport = transport_name, "serve starting");

        let router = Arc::new(self.router);
        let root = self.root;
        let scopes = self.scopes;
        let connection_order = self.connection_order;
        let request_order = self.request_order;
        let mut shutdown = self.shutdown;

        loop {
            tokio::select! {
                result = transport.accept() => {
                    match result {
                        Ok(conn) => {
                            debug!(peer = ?conn.peer().addr, "connection accepted, spawning task");

                            let router = Arc::clone(&router);
                            let root = Arc::clone(&root);
                            let scopes = Arc::clone(&scopes);
                            let connection_order = Arc::clone(&connection_order);
                            let request_order = Arc::clone(&request_order);

                            tokio::spawn(async move {
                                serve_connection(
                                    conn, router, root, scopes, connection_order, request_order,
                                )
                                .await;
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
    level = "debug",
    skip_all,
    fields(peer = ?conn.peer().addr),
    name = "connection"
)]
async fn serve_connection<C: Connection>(
    mut conn: C,
    router: Arc<RpcRouter>,
    root: Arc<ScopeContainer>,
    scopes: Arc<ScopeRegistry>,
    connection_order: Arc<Vec<ComponentDescriptor>>,
    request_order: Arc<Vec<ComponentDescriptor>>,
) {
    debug!("connection established");

    // Seed the peer (by value — the framework's connection-scoped injectable) and
    // build the connection scope (e.g. authenticated session, checked-out DB
    // handle). A failed factory closes the connection.
    let peer = conn.peer().clone();
    let seed = BoxedComponent {
        ty: TypeDescriptor::of::<PeerInfo>("PeerInfo"),
        value: Box::new(peer),
    };

    let connection_scope = match ScopeContainer::open_child(
        ComponentScope::Connection,
        root,
        Arc::clone(&scopes),
        &connection_order,
        vec![seed],
    )
    .await
    {
        Ok(scope) => scope,

        Err(e) => {
            error!(error = %e, "connection scope build failed, closing");
            return;
        }
    };

    let mut tasks: JoinSet<()> = JoinSet::new();

    debug!("connection ready");

    loop {
        match conn.recv().await {
            Ok(Some((call, responder))) => {
                let path = call.path;
                let router = Arc::clone(&router);
                let connection_scope = Arc::clone(&connection_scope);
                let scopes = Arc::clone(&scopes);
                let request_order = Arc::clone(&request_order);

                debug!(%path, "dispatching call");

                tasks.spawn(drive_call(
                    path,
                    call.payload,
                    call.requests,
                    call.cancel,
                    connection_scope,
                    scopes,
                    request_order,
                    responder,
                    router,
                ));
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

    // The connection (and its call table) is dropped here, cancelling in-flight
    // calls via their tokens; abort any handler tasks still winding down.
    tasks.abort_all();

    debug!("connection ended");
}

/// Drives one call to completion on its own task: build its request scope,
/// dispatch, then pump the outcome into the matching responder — a single reply
/// for unary calls, or a stream of items terminated by `finish`/`error` for
/// streaming calls.
#[allow(clippy::too_many_arguments)]
async fn drive_call<R>(
    path: String,
    payload: Vec<u8>,
    requests: Option<mpsc::Receiver<Vec<u8>>>,
    cancel: CancellationToken,
    connection_scope: Arc<ScopeContainer>,
    scopes: Arc<ScopeRegistry>,
    request_order: Arc<Vec<ComponentDescriptor>>,
    responder: R,
    router: Arc<RpcRouter>,
) where
    R: Respond + RespondStream + Send + 'static,
{
    let request_scope = match ScopeContainer::open_child(
        ComponentScope::Request,
        connection_scope,
        scopes,
        &request_order,
        Vec::new(),
    )
    .await
    {
        Ok(scope) => scope,

        Err(e) => {
            error!(%path, error = %e, "request scope build failed");
            let response = ErrorResponse::from(e);
            let _ = responder
                .respond(CallResult::Err {
                    code: response.code,
                    body: response.body,
                })
                .await;

            return;
        }
    };

    let ctx = RpcCallContext::new(payload, request_scope, requests, cancel);

    match router.dispatch(&path, ctx).await {
        Ok(RpcOutcome::Unary(RpcResponse { payload })) => {
            debug!(%path, "call succeeded");

            if let Err(e) = responder.respond(CallResult::Ok(payload)).await {
                warn!(%path, error = %e, "failed to send response");
            }
        }

        Ok(RpcOutcome::Stream(mut stream)) => {
            debug!(%path, "streaming response");

            let mut sink = responder.into_sink();

            loop {
                match stream.next().await {
                    Some(Ok(item)) => {
                        if let Err(e) = sink.send(item).await {
                            warn!(%path, error = %e, "failed to send stream item");

                            return;
                        }
                    }

                    Some(Err(e)) => {
                        warn!(%path, code = ?e.code, "stream handler errored");
                        let _ = sink.error(e.code, e.body).await;

                        return;
                    }

                    None => break,
                }
            }

            if let Err(e) = sink.finish().await {
                warn!(%path, error = %e, "failed to finish stream");
            }
        }

        Err(e) => {
            warn!(%path, code = ?e.code, "call returned error");

            if let Err(e) = responder
                .respond(CallResult::Err {
                    code: e.code,
                    body: e.body,
                })
                .await
            {
                warn!(%path, error = %e, "failed to send error response");
            }
        }
    }
}
