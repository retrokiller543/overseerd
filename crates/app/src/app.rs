use std::{
    any::TypeId,
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};

use futures::FutureExt;
use overseerd_config::{
    CONFIG_RELOADER_ID, CONFIG_RELOADER_NAME, ConfigBinding, ConfigManager, ConfigProperties,
    ConfigReloader, ConfigStore, ReloadTriggers, spawn_reload_triggers, stop_reload_triggers,
};
use overseerd_core::{
    Descriptor, ResolverCtx, ResolverSet, Singleton as SingletonScope, TypeDescriptor,
};
use overseerd_di::{
    BoxedComponent, Component, ComponentDescriptor, Injectable, RootResolver, ScopeContainer,
    ScopeRegistry, root_resolver_descriptor,
};
use overseerd_dirs::{Cache, Config, Data, Dir, DirKind, DirectoriesManager, Runtime, State, Tmp};
use overseerd_hooks::{
    HOOK_MANAGER_ID, HOOK_MANAGER_NAME, HookDescriptor, HookKind, HookManager, Shutdown, Startup,
};
use tracing::{debug, error, info};

use crate::error::Error;
use crate::lifecycle::{ShutdownHandle, ShutdownSignal};
use crate::plugin::{
    ApplicationPluginRegistrar, EffectivePluginPlan, Plugin, PluginCatalog, PluginWithOptions,
    ProtocolPluginRegistrar,
};
use crate::protocol::{
    PreBuildContext, PreparedProtocol, ProtocolDefinition, ProtocolRuntime, Serve,
    ValidationContext,
};
use crate::registry::AppRegistry;
use crate::runtime::{AppRuntime, RuntimeScopePlan};
use crate::scope::{PreparedScopeTopology, ScopePlan, SeedDestination};

/// The framework-provided singleton injectable for triggering graceful shutdown.
static SHUTDOWN_HANDLE_DESCRIPTOR: ComponentDescriptor = ComponentDescriptor::manual(
    crate::builtins::shutdown::SHUTDOWN_HANDLE_ID,
    crate::builtins::shutdown::SHUTDOWN_HANDLE_NAME,
    TypeDescriptor::of::<ShutdownHandle>(crate::builtins::shutdown::SHUTDOWN_HANDLE_NAME),
    &SingletonScope,
);

/// The framework-provided singleton injectable for triggering a config reload.
static CONFIG_RELOADER_DESCRIPTOR: ComponentDescriptor = ComponentDescriptor::manual(
    CONFIG_RELOADER_ID,
    CONFIG_RELOADER_NAME,
    TypeDescriptor::of::<ConfigReloader>(CONFIG_RELOADER_NAME),
    &SingletonScope,
);

/// The framework-provided singleton injectable that runs lifecycle/event hooks.
static HOOK_MANAGER_DESCRIPTOR: ComponentDescriptor = ComponentDescriptor::manual(
    HOOK_MANAGER_ID,
    HOOK_MANAGER_NAME,
    TypeDescriptor::of::<HookManager>(HOOK_MANAGER_NAME),
    &SingletonScope,
);

/// Assembles an [`App`] from an explicit set of components and a protocol definition.
///
/// Generic over the [`ProtocolDefinition`] it prepares. The agnostic builder methods (config,
/// components, directories, auto-discovery) live here; protocol-specific methods come from
/// an extension trait (e.g. `RpcAppBuilder` in `overseerd-rpc`), so the same builder serves
/// any protocol.
pub struct AppBuilder<D: ProtocolDefinition> {
    name: String,
    registry: AppRegistry,
    instances: Vec<BoxedComponent>,
    config_source: Option<ConfigManager>,
    /// Whether link-time application, protocol, config, and plugin discovery is enabled.
    auto_discovery_enabled: bool,
    dirs: Option<DirectoriesManager>,
    plugins: PluginCatalog,
    /// The selected protocol definition and its accumulated configuration.
    protocol: D,
}

/// A validated application assembly awaiting runtime component construction.
///
/// Preparation resolves registrations, configuration, protocol validation, and scope plans
/// without invoking factory-backed application components or constructing the served protocol.
pub struct PreparedApp<D: ProtocolDefinition> {
    name: String,
    registry: AppRegistry,
    instances: Vec<BoxedComponent>,
    protocol: D::Prepared,
    plugin_plan: EffectivePluginPlan,
    shutdown: ShutdownSignal,
    root_resolver: RootResolver,
    hook_manager: HookManager,
    reloader: ConfigReloader,
    reload_triggers: ReloadTriggers,
    resolved: Arc<[ComponentDescriptor]>,
    singletons: Vec<ComponentDescriptor>,
    scope_registry: Arc<ScopeRegistry>,
    scope_topology: Arc<PreparedScopeTopology>,
    scope_orders: Arc<HashMap<overseerd_core::ScopeId, Vec<ComponentDescriptor>>>,
    seed_destinations: Arc<HashMap<std::any::TypeId, SeedDestination>>,
    resolvers: ResolverSet,
}

impl<D: ProtocolDefinition> AppBuilder<D> {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registry: AppRegistry::default(),
            instances: Vec::new(),
            config_source: None,
            auto_discovery_enabled: false,
            dirs: None,
            plugins: PluginCatalog::new(),
            protocol: D::default(),
        }
    }

    /// Mutable access to the protocol definition, for protocol-specific extension traits.
    pub fn protocol_mut(&mut self) -> &mut D {
        &mut self.protocol
    }

    /// Mutable access to the agnostic registry, for protocol-specific extension traits
    /// that also register components (e.g. a service's component descriptor).
    pub fn registry_mut(&mut self) -> &mut AppRegistry {
        &mut self.registry
    }

    /// Supplies the merged configuration the app binds its `Cfg<T>` injections from. If
    /// omitted, the app loads config from its `Dir<Config>` directory.
    pub fn config_source<F>(mut self, config: ConfigManager<F>) -> Self {
        self.config_source = Some(config.into_dynamic());

        self
    }

    /// Supplies the [`DirectoriesManager`] the app seeds its `Dir<K>` injectables from.
    pub fn directories(mut self, dirs: DirectoriesManager) -> Self {
        self.dirs = Some(dirs);

        self
    }

    /// Binds config type `T` to the subtree at `path`, injectable as `Cfg<T>`.
    pub fn config<T: ConfigProperties>(mut self, path: impl Into<String>) -> Self {
        self.registry
            .config_bindings
            .push(ConfigBinding::of::<T>(path));

        self
    }

    /// Registers a pre-built singleton instance, holding it until the container is built.
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

    /// Merges every link-time-registered component and provider descriptor (and the
    /// protocol's own variants) into the app, and enables
    /// config auto-discovery.
    pub fn auto_discover(mut self) -> Self {
        let discovered = AppRegistry::collect();

        self.registry.components.extend(discovered.components);
        self.registry.providers.extend(discovered.providers);
        self.auto_discovery_enabled = true;
        self.protocol.auto_discover();

        self
    }

    /// Registers component type `T` for construction from its statically-known descriptor.
    pub fn component<T>(mut self) -> Self
    where
        T: Descriptor<ComponentDescriptor>,
    {
        self.registry
            .components
            .push(<T as Descriptor<ComponentDescriptor>>::DESCRIPTOR);

        self
    }

    /// Manually register a raw component descriptor for construction during build.
    pub fn component_descriptor(mut self, descriptor: &'static ComponentDescriptor) -> Self {
        self.registry.components.push(*descriptor);

        self
    }

    /// Retains plugin type `P` for deterministic composition during preparation.
    pub fn register_plugin<P: Plugin + Default>(mut self) -> Self {
        self.plugins.with_plugin(P::default());

        self
    }

    /// Retains plugin type `P` constructed synchronously from explicit options.
    pub fn register_plugin_with_options<P: PluginWithOptions>(
        mut self,
        options: P::Options,
    ) -> Self {
        self.plugins.with_plugin(P::from_options(options));

        self
    }

    /// Retains a supplied plugin instance for deterministic composition during preparation.
    pub fn with_plugin<P: Plugin>(mut self, plugin: P) -> Self {
        self.plugins.with_plugin(plugin);

        self
    }

    pub(crate) fn with_plugin_declarations(
        mut self,
        declarations: impl FnOnce(&mut ApplicationPluginRegistrar),
    ) -> Self {
        self.plugins.declare(declarations);

        self
    }

    /// Registers and validates the application without constructing ordinary components.
    pub fn prepare(self) -> Result<PreparedApp<D>, D::Error> {
        debug!(target: "overseerd::app", app = %self.name, "building app");

        let mut registry = self.registry;
        let mut instances = self.instances;
        let mut protocol = self.protocol;
        let mut protocol_plugins = ProtocolPluginRegistrar::new(D::ID);

        D::register_plugins(&mut protocol_plugins);

        let plugin_plan = self
            .plugins
            .freeze(protocol_plugins, self.auto_discovery_enabled)?;
        let plugin_plan = plugin_plan.lower(&mut registry);

        // Consumed by `serve`/`run`; its handle is seeded as a framework injectable.
        let shutdown = ShutdownSignal::new();

        // A run-time handle to the finished root container, seeded now (empty) and attached
        // once the root is built, so a singleton can resolve from the container after startup.
        let root_resolver = RootResolver::new();

        // The protocol definition contributes its DI descriptors (for RPC, the connection-scoped
        // `PeerInfo` injectable) before validation.
        protocol.register(&mut registry);

        // Directories are framework-provided singletons: a manager plus one `Dir<K>` per kind.
        let dirs = match self.dirs {
            Some(dirs) => dirs,
            None => DirectoriesManager::try_for_app(&self.name).map_err(Error::Directories)?,
        };
        seed_directories(&dirs, &mut registry, &mut instances);

        // Other framework singletons (the shutdown handle, the root resolver).
        seed_builtins(&shutdown, &root_resolver, &mut registry, &mut instances);

        // The config reloader and hook manager are always available; their instances are
        // seeded below, once the config slots and collected hooks exist.
        registry.components.push(CONFIG_RELOADER_DESCRIPTOR);
        registry.components.push(HOOK_MANAGER_DESCRIPTOR);

        protocol.pre_build(&mut PreBuildContext::new(&mut registry, &mut instances))?;

        // Finalize the config manager. Auto-discovery (gated on the builder's `auto_discover`)
        // registers `#[config(path)]` types and seeds defaults; explicit bindings fold in next.
        let explicit_bindings = std::mem::take(&mut registry.config_bindings);
        let mut tree = match self.config_source {
            Some(config) => config,
            None => ConfigManager::load_in(&dirs.config_path(), &[]).map_err(Error::from)?,
        }
        .with_directories(&dirs);

        if self.auto_discovery_enabled {
            tree = tree.auto_discover();
        }

        for binding in explicit_bindings {
            tree.register_binding(binding);
        }

        registry.config_bindings = tree.bindings().to_vec();

        let scope_topology = Arc::new(D::SCOPE_TOPOLOGY.prepare().map_err(Error::from)?);

        registry.validate_with_scope_topology(&scope_topology)?;

        // Collapse to the effective component set (explicit factories override defaults).
        let resolved = registry.resolved_components()?;
        registry.components = resolved.clone();

        // Collect every component's `#[hook]` methods into the hook manager.
        let hooks: Vec<HookDescriptor> = resolved
            .iter()
            .flat_map(|component| (component.hooks)().iter().copied())
            .collect();
        let hook_manager = HookManager::new(hooks);
        instances.push(BoxedComponent {
            ty: TypeDescriptor::of::<HookManager>(HOOK_MANAGER_NAME),
            value: Box::new(Injectable::into_stored(hook_manager.clone())),
        });

        let scopes = ScopePlan::partition(&resolved, &registry.providers, &scope_topology)?;

        // Build the config store — every bound `Cfg<T>` value, plus the reload slots.
        let (config_store, reload_slots) = ConfigStore::build(&tree).map_err(Error::from)?;
        let config_store = Arc::new(config_store);

        let protocol = protocol.prepare(&ValidationContext::new(
            &self.name,
            &registry,
            config_store.as_ref(),
            &plugin_plan,
        ))?;

        let mut resolvers = ResolverSet::new();
        resolvers.insert(config_store);

        let reload_triggers = tree.triggers();

        let reloader = ConfigReloader::new(tree, reload_slots, hook_manager.clone());
        instances.push(BoxedComponent {
            ty: TypeDescriptor::of::<ConfigReloader>(CONFIG_RELOADER_NAME),
            value: Box::new(Injectable::into_stored(reloader.clone())),
        });

        let scope_registry = Arc::new(ScopeRegistry::new(
            scopes.transient,
            resolved
                .iter()
                .filter(|component| component.effective_factory().ok().flatten().is_some())
                .map(|component| (component.ty.type_id, *component))
                .collect(),
            registry.providers.clone(),
            registry
                .component_registry()
                .provider_order(&resolved)
                .map_err(Error::from)?,
        ));

        Ok(PreparedApp {
            name: self.name,
            registry,
            instances,
            protocol,
            plugin_plan,
            shutdown,
            root_resolver,
            hook_manager,
            reloader,
            reload_triggers,
            resolved: Arc::from(resolved),
            singletons: scopes.singletons,
            scope_registry,
            scope_topology,
            scope_orders: Arc::new(scopes.orders),
            seed_destinations: Arc::new(scopes.seed_destinations),
            resolvers,
        })
    }

    /// Validates, constructs, and finalizes a ready-to-run [`App`].
    pub async fn build(self) -> Result<App<D>, D::Error> {
        self.prepare()?.build().await
    }
}

impl<D: ProtocolDefinition> PreparedApp<D> {
    /// The configured application name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The validated effective application registry.
    pub fn registry(&self) -> &AppRegistry {
        &self.registry
    }

    /// The validated protocol state awaiting runtime construction.
    pub fn protocol(&self) -> &D::Prepared {
        &self.protocol
    }

    /// The immutable effective plugin plan lowered during preparation.
    pub fn plugin_plan(&self) -> &EffectivePluginPlan {
        &self.plugin_plan
    }

    /// The stable identity of the selected protocol definition.
    pub const fn protocol_id(&self) -> crate::ProtocolId {
        D::ID
    }

    /// The validated protocol-owned scope topology used for planning and runtime opening.
    pub fn scope_topology(&self) -> &PreparedScopeTopology {
        &self.scope_topology
    }

    /// Constructs ordinary components and finalizes the application protocol.
    pub async fn build(self) -> Result<App<D>, D::Error> {
        let PreparedApp {
            name,
            registry,
            instances,
            protocol,
            plugin_plan,
            shutdown,
            root_resolver,
            hook_manager,
            reloader,
            reload_triggers,
            resolved,
            singletons,
            scope_registry,
            scope_topology,
            scope_orders,
            seed_destinations,
            resolvers,
        } = self;

        let root = ScopeContainer::build_root(
            &singletons,
            instances,
            resolvers,
            Arc::clone(&scope_registry),
        )
        .await
        .map_err(Error::from)?;

        // Hooks resolve their `&self` receiver through the root container.
        let hook_ctx: Arc<dyn ResolverCtx + Send + Sync> = root.clone();
        hook_manager.attach(hook_ctx);

        // The root resolver hands the finished root to any singleton that needs to resolve
        // from the container at run time (kept as a `Weak`, so it adds no reference cycle).
        root_resolver.attach(&root);

        info!(target: "overseerd::app",
            app = %name,
            components = registry.components.len(),
            "app built"
        );

        let runtime = AppRuntime::new(
            Arc::from(name.as_str()),
            root,
            scope_registry,
            RuntimeScopePlan::new(scope_topology, scope_orders, seed_destinations),
            resolved,
            hook_manager,
        );

        // Hand off to the prepared protocol: it constructs the served runtime.
        let protocol = protocol.build(&runtime)?;

        Ok(App {
            name,
            registry,
            runtime,
            protocol,
            plugin_plan,
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

/// Seeds the [`DirectoriesManager`] and one `Dir<K>` per kind as singleton instances.
fn seed_directories(
    dirs: &DirectoriesManager,
    registry: &mut AppRegistry,
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

/// Seeds the framework builtin singletons — the [`ShutdownHandle`] and the [`RootResolver`]
/// (seeded unattached; [`RootResolver::attach`] wires it to the finished root after build).
fn seed_builtins(
    shutdown: &ShutdownSignal,
    root_resolver: &RootResolver,
    registry: &mut AppRegistry,
    instances: &mut Vec<BoxedComponent>,
) {
    registry.components.push(SHUTDOWN_HANDLE_DESCRIPTOR);
    instances.push(BoxedComponent {
        ty: TypeDescriptor::of::<ShutdownHandle>(<ShutdownHandle as Component>::NAME),
        value: Box::new(shutdown.handle()),
    });

    registry.components.push(root_resolver_descriptor());
    instances.push(BoxedComponent {
        ty: TypeDescriptor::of::<RootResolver>(<RootResolver as Component>::NAME),
        value: Box::new(Injectable::into_stored(root_resolver.clone())),
    });
}

/// Seeds one `Dir<K>` as a singleton instance.
fn seed_dir<K: DirKind>(
    dirs: &DirectoriesManager,
    registry: &mut AppRegistry,
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

/// A fully assembled app, ready to serve its protocol.
///
/// Holds the agnostic [`AppRuntime`] (DI container, scope orders, hooks) and the built
/// [`ProtocolRuntime`], plus the shutdown signal and config reloader the serve envelope drives.
pub struct App<D: ProtocolDefinition> {
    pub name: String,
    pub registry: AppRegistry,
    runtime: AppRuntime,
    protocol: <D::Prepared as PreparedProtocol>::Runtime,
    plugin_plan: EffectivePluginPlan,
    shutdown: ShutdownSignal,
    reloader: ConfigReloader,
    reload_triggers: ReloadTriggers,
}

impl<D: ProtocolDefinition> fmt::Debug for App<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("App")
            .field("name", &self.name)
            .field("components", &self.registry.components.len())
            .finish_non_exhaustive()
    }
}

impl<D: ProtocolDefinition> fmt::Display for App<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "App: {}", self.name)?;
        write!(f, "{}", self.registry)?;

        Ok(())
    }
}

impl<D: ProtocolDefinition> App<D> {
    /// Starts building an app for protocol definition `D`. Most protocols expose a pinned
    /// alias (e.g. `overseerd_rpc::App = App<Rpc>`) so `App::builder(name)` resolves
    /// without a turbofish.
    pub fn builder(name: impl Into<String>) -> AppBuilder<D> {
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

    /// The installed protocol.
    pub fn protocol(&self) -> &<D::Prepared as PreparedProtocol>::Runtime {
        &self.protocol
    }

    /// The immutable effective plugin plan used to assemble this application.
    pub fn plugin_plan(&self) -> &EffectivePluginPlan {
        &self.plugin_plan
    }

    /// The stable identity of the selected protocol definition.
    pub const fn protocol_id(&self) -> crate::ProtocolId {
        D::ID
    }

    /// Returns a handle that can trigger graceful shutdown from any spawned task.
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        self.shutdown.handle()
    }

    /// A handle that re-reads configuration and re-publishes the changed bindings.
    pub fn config_reloader(&self) -> ConfigReloader {
        self.reloader.clone()
    }

    /// The hook manager, for running lifecycle/event hooks by kind.
    pub fn hook_manager(&self) -> HookManager {
        self.runtime.hooks().clone()
    }

    /// Serves the app's protocol over `endpoint` until ctrl-c or a shutdown signal.
    ///
    /// The agnostic envelope: runs startup hooks, spawns config-reload triggers, bridges
    /// ctrl-c to the shutdown signal, then hands the runtime + shutdown signal to the
    /// protocol's [`Serve`] impl. Shutdown hooks run on the way out.
    pub async fn serve<E>(
        self,
        endpoint: E,
    ) -> Result<(), <<D::Prepared as PreparedProtocol>::Runtime as ProtocolRuntime>::Error>
    where
        <D::Prepared as PreparedProtocol>::Runtime: Serve<E>,
        <<D::Prepared as PreparedProtocol>::Runtime as ProtocolRuntime>::Error: From<crate::Error>,
    {
        let App {
            runtime,
            protocol,
            shutdown,
            reloader,
            reload_triggers,
            ..
        } = self;

        let started = match run_startup(runtime.hooks()).await {
            Ok(started) => started,
            Err((error, started)) => {
                run_shutdown(runtime.hooks(), &started).await;

                return Err(error.into());
            }
        };

        let trigger_tasks = spawn_reload_triggers(reloader, reload_triggers);

        // Bridge ctrl-c to the shutdown signal so every protocol's loop only watches `shutdown`.
        let shutdown_handle = shutdown.handle();
        let ctrlc = tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                info!(target: "overseerd::app", "ctrl-c received, shutting down");
                shutdown_handle.shutdown();
            }
        });

        let result =
            std::panic::AssertUnwindSafe(protocol.serve(runtime.clone(), shutdown, endpoint))
                .catch_unwind()
                .await;

        ctrlc.abort();
        let _ = ctrlc.await;

        stop_reload_triggers(trigger_tasks).await;

        run_shutdown(runtime.hooks(), &started).await;

        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Waits for ctrl-c or a shutdown signal without serving any endpoint.
    pub async fn run(self) -> crate::Result<()> {
        let App {
            runtime,
            mut shutdown,
            reloader,
            reload_triggers,
            ..
        } = self;

        let started = match run_startup(runtime.hooks()).await {
            Ok(started) => started,
            Err((error, started)) => {
                run_shutdown(runtime.hooks(), &started).await;

                return Err(error);
            }
        };

        let trigger_tasks = spawn_reload_triggers(reloader, reload_triggers);

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = shutdown.wait() => {},
        }

        stop_reload_triggers(trigger_tasks).await;

        run_shutdown(runtime.hooks(), &started).await;

        Ok(())
    }
}

/// Runs startup hooks sequentially, returning the components whose startup fully
/// succeeded. On failure the list lets the caller pair shutdown only with work that
/// actually started.
async fn run_startup(
    hooks: &HookManager,
) -> Result<HashSet<TypeId>, (crate::Error, HashSet<TypeId>)> {
    let mut started = HashSet::new();

    for (component, result) in hooks.run_until_error::<Startup>(&(), |_| true).await {
        let component_ty = component.type_id;

        match result {
            Ok(()) => {
                started.insert(component_ty);
            }
            Err(error) => {
                error!(
                    target: "overseerd::app",
                    hook = Startup::NAME,
                    component = %component.name,
                    %error,
                    "lifecycle hook failed"
                );

                return Err((error.into(), started));
            }
        }
    }

    Ok(started)
}

/// Runs shutdown hooks for components with no startup hook and for components whose
/// startup hook completed successfully. Errors are logged and cleanup continues.
async fn run_shutdown(hooks: &HookManager, started: &HashSet<TypeId>) {
    for (component, result) in hooks
        .run::<Shutdown>(&(), |hook| {
            let component_ty = hook.component_ty.type_id;

            !hooks.component_has::<Startup>(component_ty) || started.contains(&component_ty)
        })
        .await
    {
        if let Err(error) = result {
            error!(
                target: "overseerd::app",
                hook = Shutdown::NAME,
                component = %component.name,
                %error,
                "lifecycle hook failed"
            );
        }
    }
}

#[cfg(test)]
mod tests;
