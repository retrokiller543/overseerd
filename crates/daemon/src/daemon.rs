use std::{any::TypeId, collections::HashSet, fmt, sync::Arc};

use overseerd_config::{
    CONFIG_RELOADER_ID, CONFIG_RELOADER_NAME, ConfigBinding, ConfigManager, ConfigProperties,
    ConfigReloader, ConfigStore, ReloadTriggers, spawn_reload_triggers,
};
use overseerd_core::{
    Descriptor, ResolverCtx, ResolverSet, Scope, Singleton as SingletonScope, TypeDescriptor,
};
use overseerd_di::{
    BoxedComponent, Component, ComponentDescriptor, Injectable, ProviderDescriptor, ScopeContainer,
    ScopeRegistry, ServiceComponent, topological_sort,
};
use overseerd_dirs::{Cache, Config, Data, Dir, DirKind, DirectoriesManager, Runtime, State, Tmp};
use overseerd_hooks::{
    HOOK_MANAGER_ID, HOOK_MANAGER_NAME, HookDescriptor, HookKind, HookManager, Shutdown, Startup,
};
use overseerd_transport::PeerInfo;
use tower::{Layer, Service};
use tracing::{debug, error, info};

use crate::scope::{Connection as ConnectionScope, Request as RequestScope};
use crate::{
    descriptors::{RpcOutcome, ServiceDescriptor},
    extract::ErrorResponse,
    lifecycle::{ShutdownHandle, ShutdownSignal},
    middleware::{ErrorHandler, Guard, GuardLayer, RouterService, RpcRequest, RpcService},
    protocol::{Plugin, ProtocolPlugin, Rpc, Serve},
    registry::DescriptorRegistry,
    router::RpcRouter,
    runtime::AppRuntime,
};

/// A registered middleware step: wraps the current dispatch service in one more
/// layer, returning the re-erased service. Collected in registration order and
/// applied outermost-first when the daemon is built.
type LayerApplier = Box<dyn FnOnce(RpcService) -> RpcService + Send>;

/// The framework-provided connection-scoped injectable for the remote peer.
///
/// Seeded into every connection scope with the actual `PeerInfo`, so a
/// connection-scoped component can depend on `Arc<PeerInfo>` (e.g. to authenticate
/// in its constructor) — the DI-native replacement for the old `on_connect` hook.
static PEER_INFO_DESCRIPTOR: ComponentDescriptor = ComponentDescriptor::manual(
    "__overseerd_peer_info",
    "PeerInfo",
    TypeDescriptor::of::<PeerInfo>("PeerInfo"),
    &ConnectionScope,
);

/// The framework-provided singleton injectable for triggering graceful shutdown.
///
/// Seeded into the root scope with the daemon's own [`ShutdownHandle`] (a by-value,
/// `Arc`-backed clone), so any component or handler can inject it and call
/// `shutdown()`. The receiving [`ShutdownSignal`] is consumed by `serve`/`run` and
/// is never exposed through DI.
static SHUTDOWN_HANDLE_DESCRIPTOR: ComponentDescriptor = ComponentDescriptor::manual(
    crate::builtins::shutdown::SHUTDOWN_HANDLE_ID,
    crate::builtins::shutdown::SHUTDOWN_HANDLE_NAME,
    TypeDescriptor::of::<ShutdownHandle>(crate::builtins::shutdown::SHUTDOWN_HANDLE_NAME),
    &SingletonScope,
);

/// The framework-provided singleton injectable for triggering a config reload.
///
/// Seeded into the root scope with a [`ConfigReloader`] over the daemon's config
/// manager and its bound slots, so any component or handler can inject it and call
/// `reload()`.
static CONFIG_RELOADER_DESCRIPTOR: ComponentDescriptor = ComponentDescriptor::manual(
    CONFIG_RELOADER_ID,
    CONFIG_RELOADER_NAME,
    TypeDescriptor::of::<ConfigReloader>(CONFIG_RELOADER_NAME),
    &SingletonScope,
);

/// The framework-provided singleton injectable that runs lifecycle/event hooks.
///
/// Seeded into the root scope with a [`HookManager`] over every component's `#[hook]`
/// methods, so any component or handler can inject it; the
/// [`ConfigReloader`](overseerd_config::ConfigReloader) holds it to fire reload hooks.
static HOOK_MANAGER_DESCRIPTOR: ComponentDescriptor = ComponentDescriptor::manual(
    HOOK_MANAGER_ID,
    HOOK_MANAGER_NAME,
    TypeDescriptor::of::<HookManager>(HOOK_MANAGER_NAME),
    &SingletonScope,
);

/// The native RPC protocol plugin.
///
/// Accumulates the RPC-specific builder state (middleware layers and the global error
/// handler), seeds the connection-scoped `PeerInfo` injectable via [`Plugin::register`],
/// and builds the [`Rpc`] protocol — the router wrapped by the middleware stack — via
/// [`ProtocolPlugin::build`].
pub struct RpcPlugin {
    layers: Vec<LayerApplier>,
    error_handler: Option<Arc<dyn ErrorHandler>>,
}

impl RpcPlugin {
    fn new(layers: Vec<LayerApplier>, error_handler: Option<Arc<dyn ErrorHandler>>) -> Self {
        Self {
            layers,
            error_handler,
        }
    }
}

impl Plugin for RpcPlugin {
    fn register(&self, registry: &mut DescriptorRegistry) {
        registry.components.push(PEER_INFO_DESCRIPTOR);
    }
}

impl ProtocolPlugin for RpcPlugin {
    type Protocol = Rpc;
    type Error = crate::Error;

    const SCOPES: &'static [&'static dyn Scope] = &[&ConnectionScope, &RequestScope];

    fn build(self, _runtime: &AppRuntime, registry: &DescriptorRegistry) -> crate::Result<Rpc> {
        let router = Arc::new(RpcRouter::from_registry(registry));

        // Fold the registered layers onto the terminal router service. Appliers are
        // pushed in registration order, so applying them in reverse makes the
        // first-registered layer the outermost wrapper.
        let mut service: RpcService = RpcService::new(RouterService::new(Arc::clone(&router)));

        for applier in self.layers.into_iter().rev() {
            service = applier(service);
        }

        Ok(Rpc::new(router, service, self.error_handler))
    }
}

/// Assembles an App from an explicit set of components and services.
pub struct AppBuilder {
    name: String,
    registry: DescriptorRegistry,
    instances: Vec<BoxedComponent>,
    config_source: Option<ConfigManager>,
    /// Whether `auto_discover` was called: config auto-registration is gated on it, so a
    /// daemon assembled without `auto_discover` binds only its explicit `config::<T>` types.
    auto_discover_configs: bool,
    dirs: Option<DirectoriesManager>,
    layers: Vec<LayerApplier>,
    error_handler: Option<Arc<dyn ErrorHandler>>,
}

impl AppBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registry: DescriptorRegistry::default(),
            instances: Vec::new(),
            config_source: None,
            auto_discover_configs: false,
            dirs: None,
            layers: Vec::new(),
            error_handler: None,
        }
    }

    /// Supplies the merged configuration the daemon binds its `Cfg<T>` injections
    /// from. Built by the application (typically in `main`, so its values can also
    /// configure the transport) and handed in here. The format is erased — the tree
    /// is already parsed — but its format tag is retained for reload.
    ///
    /// If omitted, the daemon loads config from its `Dir<Config>` directory.
    pub fn config_source<F>(mut self, config: ConfigManager<F>) -> Self {
        self.config_source = Some(config.into_dynamic());

        self
    }

    /// Supplies the [`DirectoriesManager`] the daemon seeds its `Dir<K>` injectables
    /// from (and which the default config loader reads from). If omitted, one is
    /// constructed for the daemon's name.
    pub fn directories(mut self, dirs: DirectoriesManager) -> Self {
        self.dirs = Some(dirs);

        self
    }

    /// Binds config type `T` to the subtree at `path`, injectable as `Cfg<T>`
    /// selected by that path. The same type may be bound at several paths. This is
    /// the explicit counterpart to auto-registration via
    /// `#[config(path = "..")]`.
    pub fn config<T: ConfigProperties>(mut self, path: impl Into<String>) -> Self {
        self.registry
            .config_bindings
            .push(ConfigBinding::of::<T>(path));

        self
    }

    /// Registers a pre-built singleton instance (e.g. a stateful service
    /// constructed by hand). Synthesizes a `factory: None` descriptor in the
    /// registry (so the declaration is visible and dependencies validate) and
    /// holds the instance until the container is built. The type's identity
    /// comes from its `Component` impl (`#[component]` / `#[service]`).
    pub fn with_component<T: Component>(mut self, value: T) -> Self {
        self.registry
            .components
            .push(ComponentDescriptor::of::<T>());

        self.instances.push(BoxedComponent {
            ty: TypeDescriptor::of::<T>(T::NAME),
            value: Box::new(Injectable::into_stored(value.into_handle())),
        });

        self
    }

    /// Merges every link-time-registered component, service, and provider descriptor in the
    /// binary into the registry, preserving anything already registered (manual components,
    /// instances), and enables config auto-discovery.
    ///
    /// Config bindings are *not* folded in here: they are owned by the [`ConfigManager`], so
    /// this only records the intent (via a flag). At build the manager's
    /// [`auto_discover`](ConfigManager::auto_discover) runs, which both registers the
    /// `#[config(path = "..")]` types and seeds their defaults. A daemon built *without*
    /// `auto_discover` therefore auto-registers no config types — only explicit
    /// [`config::<T>`](Self::config) bindings.
    pub fn auto_discover(mut self) -> Self {
        let discovered = DescriptorRegistry::collect();

        self.registry.components.extend(discovered.components);
        self.registry.services.extend(discovered.services);
        self.registry.providers.extend(discovered.providers);
        self.auto_discover_configs = true;

        self
    }

    /// Registers a pre-built service singleton: its identity header (which carries
    /// the service's own RPC surface) and the instance itself (like
    /// [`with_component`](Self::with_component)).
    ///
    /// Reads the header straight from the type via [`Descriptor`], so it brings the
    /// service's RPCs with it — no separate registration. Safe to combine with
    /// [`auto_discover`](Self::auto_discover): services dedup by type at build, so a
    /// service registered both ways resolves once.
    pub fn with_service<T: ServiceComponent + Descriptor<ServiceDescriptor>>(
        mut self,
        value: T,
    ) -> Self {
        self.registry
            .services
            .push(<T as Descriptor<ServiceDescriptor>>::DESCRIPTOR);

        self.with_component(value)
    }

    /// Registers component type `T` for construction from its statically-known
    /// descriptor (its `#[component]`/`#[service]` factory), without auto-discovery.
    /// For a type built outside the DI scope, supply the value via
    /// [`with_component`](Self::with_component) instead.
    pub fn component<T>(mut self) -> Self
    where
        T: Descriptor<ComponentDescriptor>,
    {
        self.registry
            .components
            .push(<T as Descriptor<ComponentDescriptor>>::DESCRIPTOR);

        self
    }

    /// Registers service type `T` by type: its identity header (carrying its RPC
    /// surface) and its construction factory. The complete by-type counterpart to
    /// [`auto_discover`](Self::auto_discover); safe to combine with it (services dedup
    /// by type at build).
    pub fn service<T>(mut self) -> Self
    where
        T: Descriptor<ServiceDescriptor> + Descriptor<ComponentDescriptor>,
    {
        self.registry
            .services
            .push(<T as Descriptor<ServiceDescriptor>>::DESCRIPTOR);
        self.registry
            .components
            .push(<T as Descriptor<ComponentDescriptor>>::DESCRIPTOR);

        self
    }

    /// Manually register a raw component descriptor for construction during build.
    /// Prefer [`component`](Self::component) (by type), [`with_component`](Self::with_component)
    /// for instances, or the [`component`](overseerd_macros::component) macro.
    pub fn component_descriptor(mut self, descriptor: &'static ComponentDescriptor) -> Self {
        self.registry.components.push(*descriptor);

        self
    }

    /// Manually register a raw service header (prefer [`service`](Self::service) by type).
    /// The descriptor's `rpcs` pointer carries the service's RPC surface.
    pub fn service_descriptor(mut self, descriptor: &'static ServiceDescriptor) -> Self {
        self.registry.services.push(*descriptor);

        self
    }

    /// Wraps the dispatch path in a [`tower::Layer`], running on every call. Any
    /// protocol-agnostic tower layer (timeout, rate-limit, …) or a framework layer
    /// works. The first layer registered is the outermost (it sees the request
    /// first and the response last).
    pub fn middleware<L>(mut self, layer: L) -> Self
    where
        L: Layer<RpcService> + Send + 'static,
        L::Service: Service<RpcRequest, Response = RpcOutcome, Error = ErrorResponse>
            + Clone
            + Send
            + 'static,
        <L::Service as Service<RpcRequest>>::Future: Send + 'static,
    {
        self.layers
            .push(Box::new(move |inner| RpcService::new(layer.layer(inner))));

        self
    }

    /// Registers a [`Guard`] as a pre-handler admit/reject check, adapted onto the
    /// middleware stack. Equivalent to a [`middleware`](Self::middleware) of a
    /// [`GuardLayer`], ordered like any other layer.
    pub fn guard<G: Guard>(self, guard: G) -> Self {
        self.middleware(GuardLayer::new(Arc::new(guard)))
    }

    /// Sets the single global [`ErrorHandler`] applied to every error response
    /// before it reaches the caller. A later call replaces an earlier one.
    pub fn error_handler<H: ErrorHandler>(mut self, handler: H) -> Self {
        self.error_handler = Some(Arc::new(handler));

        self
    }

    /// Validates the registry, resolves all components, partitions them by scope,
    /// and builds a ready-to-run App.
    pub async fn build(self) -> crate::Result<App> {
        debug!(target: "overseerd::daemon", daemon = %self.name, "building daemon");

        let mut registry = self.registry;
        let mut instances = self.instances;

        // Created before the singleton seeding so its handle can be seeded as a
        // framework injectable. The signal itself (the receiver half) is consumed
        // by `serve`/`run` and never exposed through DI.
        let shutdown = ShutdownSignal::new();

        // The protocol plugin: it accumulates the protocol-specific builder state
        // (middleware layers, error handler) and contributes its DI descriptors here —
        // for RPC, the connection-scoped `PeerInfo` injectable, declared so dependencies
        // on it validate and it partitions into the connection scope.
        let plugin = RpcPlugin::new(self.layers, self.error_handler);
        plugin.register(&mut registry);

        // Directories are framework-provided singletons: a manager (supplied or
        // derived from the daemon name) plus one `Dir<K>` per kind, seeded so any
        // component can inject them.
        let dirs = self
            .dirs
            .unwrap_or_else(|| DirectoriesManager::for_app(&self.name));
        seed_directories(&dirs, &mut registry, &mut instances);

        // Other framework singletons (the shutdown handle) are seeded alongside the
        // directories so any component can inject them.
        seed_builtins(&shutdown, &mut registry, &mut instances);

        // The config reloader and hook manager are always available; declare them now so
        // they partition as singletons. Their instances are seeded below, once the config
        // slots and collected hooks exist.
        registry.components.push(CONFIG_RELOADER_DESCRIPTOR);
        registry.components.push(HOOK_MANAGER_DESCRIPTOR);

        // Finalize the config manager — it owns the config registry. Auto-discovery (gated on
        // the builder's `auto_discover`, so a daemon without it auto-registers no configs)
        // registers `#[config(path)]` types and seeds their defaults; explicit `config::<T>`
        // bindings are folded in next. The directory namespace is wired so defaults and
        // values may reference `${@runtime}` and friends. The registry then reads the bindings
        // back for validation and `Cfg<T>` construction.
        let explicit_bindings = std::mem::take(&mut registry.config_bindings);
        let mut tree = match self.config_source {
            Some(config) => config,
            None => ConfigManager::load_in(&dirs.config_path(), &[])?,
        }
        .with_directories(&dirs);

        if self.auto_discover_configs {
            tree = tree.auto_discover();
        }

        for binding in explicit_bindings {
            tree.register_binding(binding);
        }

        registry.config_bindings = tree.bindings().to_vec();

        registry.validate()?;

        // Collapse to the effective component set (explicit factories override
        // field-injection defaults) so the stored registry reflects what runs.
        let resolved = registry.resolved_components()?;
        registry.components = resolved.clone();

        // Collect every component's `#[hook]` methods (empty slices contribute nothing)
        // into the hook manager, seeded as a framework singleton. Its container is attached
        // once the root scope is built.
        let hooks: Vec<HookDescriptor> = resolved
            .iter()
            .flat_map(|component| (component.hooks)().iter().copied())
            .collect();
        let hook_manager = HookManager::new(hooks);
        instances.push(BoxedComponent {
            ty: TypeDescriptor::of::<HookManager>(HOOK_MANAGER_NAME),
            value: Box::new(Injectable::into_stored(hook_manager.clone())),
        });

        let scopes = ScopePlan::partition(&resolved, &registry.providers)?;

        // Build the config store — every bound `Cfg<T>` value, plus the reload slots
        // sharing their live cells. The store is a resolver, inserted into the resolver set
        // the container threads to every factory: config lives *outside* the container, so a
        // `Cfg<T>` resolves through `ctx.get_resolver::<ConfigStore>()`, not the component
        // store.
        let (config_store, reload_slots) = ConfigStore::build(&tree)?;
        let mut resolvers = ResolverSet::new();
        resolvers.insert(Arc::new(config_store));

        // Capture the manager's reload triggers before it moves into the reloader, so
        // `serve`/`run` can spawn the matching background tasks.
        let reload_triggers = tree.triggers();

        // Build the reloader over the (now finalized) manager and the slots sharing the
        // bound configs' live cells, then seed it as a framework singleton instance.
        let reloader = ConfigReloader::new(tree, reload_slots, hook_manager.clone());
        instances.push(BoxedComponent {
            ty: TypeDescriptor::of::<ConfigReloader>(CONFIG_RELOADER_NAME),
            value: Box::new(Injectable::into_stored(reloader.clone())),
        });

        let scope_registry = Arc::new(ScopeRegistry::new(
            scopes.transient,
            registry.providers.clone(),
        ));
        let root = ScopeContainer::build_root(
            &scopes.singletons,
            instances,
            resolvers,
            Arc::clone(&scope_registry),
        )
        .await?;

        // Hooks resolve their `&self` receiver through the root container (as a resolver
        // context), which only now exists.
        let hook_ctx: Arc<dyn ResolverCtx + Send + Sync> = root.clone();
        hook_manager.attach(hook_ctx);

        info!(target: "overseerd::daemon",
            daemon = %self.name,
            components = registry.components.len(),
            services = registry.services.len(),
            "daemon built"
        );

        let runtime = AppRuntime::new(
            Arc::from(self.name.as_str()),
            root,
            scope_registry,
            Arc::new(scopes.connection_order),
            Arc::new(scopes.request_order),
            scopes.needs_peer,
            hook_manager,
        );

        // Hand off to the protocol plugin: it builds the router from the validated
        // registry and folds the middleware stack into the served protocol.
        let protocol = plugin.build(&runtime, &registry)?;

        Ok(App {
            name: self.name,
            registry,
            runtime,
            protocol,
            shutdown,
            reloader,
            reload_triggers,
        })
    }
}

macro_rules! seed_dirs {
    ($dirs:ident; $registry:ident; $instances:ident; $($name:ident),*) => {
        $(seed_dir::<$name>($dirs, $registry, $instances);)*
    };
}

/// Seeds the [`DirectoriesManager`] and one `Dir<K>` per kind as singleton
/// instances, so any component can inject them.
fn seed_directories(
    dirs: &DirectoriesManager,
    registry: &mut DescriptorRegistry,
    instances: &mut Vec<BoxedComponent>,
) {
    registry
        .components
        .push(ComponentDescriptor::of::<DirectoriesManager>());
    instances.push(BoxedComponent {
        ty: TypeDescriptor::of::<DirectoriesManager>(<DirectoriesManager as Component>::NAME),
        value: Box::new(dirs.clone()),
    });

    seed_dirs!(
        dirs; registry; instances;
        Config, Data, Cache, State, Runtime, Tmp
    );
}

/// Seeds the framework builtin singletons — currently the [`ShutdownHandle`] — as
/// by-value singleton instances with their factory-less descriptors.
fn seed_builtins(
    shutdown: &ShutdownSignal,
    registry: &mut DescriptorRegistry,
    instances: &mut Vec<BoxedComponent>,
) {
    registry.components.push(SHUTDOWN_HANDLE_DESCRIPTOR);
    instances.push(BoxedComponent {
        ty: TypeDescriptor::of::<ShutdownHandle>(<ShutdownHandle as Component>::NAME),
        value: Box::new(shutdown.handle()),
    });
}

/// Seeds one `Dir<K>` as a singleton instance with its factory-less descriptor.
fn seed_dir<K: DirKind>(
    dirs: &DirectoriesManager,
    registry: &mut DescriptorRegistry,
    instances: &mut Vec<BoxedComponent>,
) {
    registry
        .components
        .push(ComponentDescriptor::of::<Dir<K>>());
    instances.push(BoxedComponent {
        ty: TypeDescriptor::of::<Dir<K>>(<Dir<K> as Component>::NAME),
        value: Box::new(dirs.dir::<K>()),
    });
}

/// The per-scope construction plan computed once at daemon build.
struct ScopePlan {
    singletons: Vec<ComponentDescriptor>,
    connection_order: Vec<ComponentDescriptor>,
    request_order: Vec<ComponentDescriptor>,
    transient: std::collections::HashMap<TypeId, ComponentDescriptor>,
    /// Whether any component depends on the framework-seeded `PeerInfo`. When
    /// false (and no connection components exist), the connection scope holds
    /// nothing and is skipped — handlers still reach the peer via the [`Peer`]
    /// extractor, which reads it off the call context rather than the scope chain.
    ///
    /// [`Peer`]: crate::extract::Peer
    needs_peer: bool,
}

impl ScopePlan {
    /// Splits the resolved components by scope and precomputes the construction
    /// order for the connection and request scopes (singletons are sorted by
    /// `build_root`; transients are constructed on demand, so need no order).
    fn partition(
        resolved: &[ComponentDescriptor],
        providers: &[ProviderDescriptor],
    ) -> crate::Result<Self> {
        let mut singletons = Vec::new();
        let mut connection_components = Vec::new();
        let mut request_components = Vec::new();
        let mut transient = std::collections::HashMap::new();

        for c in resolved {
            // A manually-seeded instance (no factory) — e.g. the framework's PeerInfo
            // — is seeded into its scope, not constructed.
            let constructable = c.effective_factory()?.is_some();

            // The Plugin step replaces this hardcoded connection/request chain with
            // the active protocol's declared `SCOPES`. Today only the four built-in
            // scopes exist, dispatched here by their label.
            if c.scope.is_transient() {
                transient.insert((c.ty.type_id)(), *c);
            } else {
                match c.scope.name() {
                    "Singleton" => singletons.push(*c),
                    "Connection" if constructable => connection_components.push(*c),
                    "Connection" => {}
                    "Request" if constructable => request_components.push(*c),
                    "Request" => {}
                    other => unreachable!("unknown component scope `{other}`"),
                }
            }
        }

        // Connection components resolve against singletons and the seeded peer. Config
        // edges impose no ordering (config resolves through an external resolver), so they
        // need not be treated as prebuilt.
        let peer_id = (PEER_INFO_DESCRIPTOR.ty.type_id)();
        let mut conn_prebuilt: HashSet<TypeId> =
            singletons.iter().map(|c| (c.ty.type_id)()).collect();
        conn_prebuilt.insert(peer_id);

        // Does any real component depend on the peer? If not, the connection scope
        // need not exist solely to hold it.
        let needs_peer = resolved.iter().any(|c| {
            (c.ty.type_id)() != peer_id
                && c.dependencies().iter().any(|d| (d.ty.type_id)() == peer_id)
        });

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
            needs_peer,
        })
    }
}

/// A fully assembled app, ready to accept connections and dispatch calls.
///
/// It holds the agnostic [`AppRuntime`] (DI container, scope orders, hooks) and the
/// built [`Protocol`](crate::protocol::Protocol) — here the native [`Rpc`] — plus the
/// shutdown signal and config reloader the serve envelope drives.
pub struct App {
    pub name: String,
    pub registry: DescriptorRegistry,
    runtime: AppRuntime,
    protocol: Rpc,
    shutdown: ShutdownSignal,
    reloader: ConfigReloader,
    reload_triggers: ReloadTriggers,
}

impl fmt::Debug for App {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("App")
            .field("name", &self.name)
            .field("components", &self.registry.components.len())
            .field("services", &self.registry.services.len())
            .field("routes", &self.protocol.route_count())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for App {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "App: {}", self.name)?;
        write!(f, "{}", self.registry)?;

        Ok(())
    }
}

impl App {
    pub fn builder(name: impl Into<String>) -> AppBuilder {
        AppBuilder::new(name)
    }

    /// The root (singleton) scope container.
    pub fn container(&self) -> &Arc<ScopeContainer> {
        self.runtime.root()
    }

    /// The protocol-facing runtime handle (DI container, scope orders, hooks).
    pub fn runtime(&self) -> &AppRuntime {
        &self.runtime
    }

    /// Returns a handle that can trigger graceful shutdown from any spawned task.
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        self.shutdown.handle()
    }

    /// A handle that re-reads configuration and re-publishes the changed bindings.
    /// The same [`ConfigReloader`] is injectable into any component or handler.
    pub fn config_reloader(&self) -> ConfigReloader {
        self.reloader.clone()
    }

    /// The hook manager, for running lifecycle/event hooks by kind. The same
    /// [`HookManager`] is injectable into any component or handler.
    pub fn hook_manager(&self) -> HookManager {
        self.runtime.hooks().clone()
    }

    /// Serves the app's protocol over `endpoint` until ctrl-c or a shutdown signal.
    ///
    /// This is the agnostic envelope: it runs startup hooks, spawns the config-reload
    /// triggers, bridges ctrl-c to the shutdown signal, and then hands the runtime and
    /// the shutdown signal to the protocol's [`Serve`] impl — for the native [`Rpc`]
    /// protocol, the per-connection / per-call dispatch loop. Shutdown hooks run on the
    /// way out.
    pub async fn serve<E>(self, endpoint: E) -> crate::Result<()>
    where
        Rpc: Serve<E>,
    {
        let App {
            runtime,
            protocol,
            shutdown,
            reloader,
            reload_triggers,
            ..
        } = self;

        // Run startup hooks before accepting work; an error aborts serve.
        run_lifecycle::<Startup>(runtime.hooks(), true).await?;

        // Spawn the configured config-reload triggers (SIGHUP / file watch); aborted on
        // shutdown below. Manual reload via the injected `ConfigReloader` is always on.
        let trigger_tasks = spawn_reload_triggers(reloader, reload_triggers);

        // Bridge ctrl-c to the shutdown signal so every protocol's loop only watches
        // `shutdown`. Aborted once serve returns.
        let shutdown_handle = shutdown.handle();
        let ctrlc = tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                info!(target: "overseerd::daemon", "ctrl-c received, shutting down");
                shutdown_handle.shutdown();
            }
        });

        let result = protocol.serve(runtime.clone(), shutdown, endpoint).await;

        ctrlc.abort();

        for task in trigger_tasks {
            task.abort();
        }

        // Graceful stop: run shutdown hooks (errors are logged, shutdown proceeds).
        run_lifecycle::<Shutdown>(runtime.hooks(), false).await.ok();

        result
    }

    /// Waits for ctrl-c or a shutdown signal without serving any transport.
    pub async fn run(self) -> crate::Result<()> {
        let App {
            runtime,
            mut shutdown,
            reloader,
            reload_triggers,
            ..
        } = self;

        run_lifecycle::<Startup>(runtime.hooks(), true).await?;

        let trigger_tasks = spawn_reload_triggers(reloader, reload_triggers);

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = shutdown.wait() => {},
        }

        for task in trigger_tasks {
            task.abort();
        }

        run_lifecycle::<Shutdown>(runtime.hooks(), false).await.ok();

        Ok(())
    }
}

/// Runs a process-lifecycle hook kind (`Startup`/`Shutdown`) over the registered hooks.
/// With `abort_on_error`, the first failing hook propagates its error (startup); otherwise
/// failures are logged and the remaining hooks still run (shutdown). A no-op when nothing
/// listens (`run` is an O(1) miss).
async fn run_lifecycle<K>(hooks: &HookManager, abort_on_error: bool) -> crate::Result<()>
where
    K: HookKind<Cx = (), Output = ()>,
{
    for (component, result) in hooks.run::<K>(&(), |_| true).await {
        if let Err(error) = result {
            error!(
                target: "overseerd::daemon",
                hook = K::NAME,
                component = %component.name,
                %error,
                "lifecycle hook failed"
            );

            if abort_on_error {
                return Err(error.into());
            }
        }
    }

    Ok(())
}
