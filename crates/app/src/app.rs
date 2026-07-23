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
    Descriptor, ResolverCtx, ResolverSet, Scope, Singleton as SingletonScope, TypeDescriptor,
};
use overseerd_di::{
    BoxedComponent, Component, ComponentDescriptor, Injectable, ProviderDescriptor, RootResolver,
    ScopeContainer, ScopeRegistry, root_resolver_descriptor, topological_sort,
};
use overseerd_dirs::{Cache, Config, Data, Dir, DirKind, DirectoriesManager, Runtime, State, Tmp};
use overseerd_hooks::{
    HOOK_MANAGER_ID, HOOK_MANAGER_NAME, HookDescriptor, HookKind, HookManager, Shutdown, Startup,
};
use tracing::{debug, error, info};

use crate::error::Error;
use crate::lifecycle::{ShutdownHandle, ShutdownSignal};
use crate::protocol::{Plugin, PreBuildContext, Protocol, ProtocolPlugin, Serve};
use crate::registry::AppRegistry;
use crate::runtime::AppRuntime;

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

/// Assembles an [`App`] from an explicit set of components and the protocol plugin.
///
/// Generic over the [`ProtocolPlugin`] it installs. The agnostic builder methods (config,
/// components, directories, auto-discovery) live here; protocol-specific methods come from
/// an extension trait (e.g. `RpcAppBuilder` in `overseerd-rpc`), so the same builder serves
/// any protocol.
pub struct AppBuilder<P: ProtocolPlugin> {
    name: String,
    registry: AppRegistry,
    instances: Vec<BoxedComponent>,
    config_source: Option<ConfigManager>,
    /// Whether `auto_discover` was called: config auto-registration is gated on it.
    auto_discover_configs: bool,
    dirs: Option<DirectoriesManager>,
    /// The protocol plugin: the builder-time accumulator for the installed protocol's
    /// own configuration.
    protocol: P,
}

/// A validated application assembly awaiting runtime component construction.
///
/// Preparation resolves registrations, configuration, protocol validation, and scope plans
/// without invoking factory-backed application components or constructing the served protocol.
pub struct PreparedApp<P: ProtocolPlugin> {
    name: String,
    registry: AppRegistry,
    instances: Vec<BoxedComponent>,
    protocol: P,
    shutdown: ShutdownSignal,
    root_resolver: RootResolver,
    hook_manager: HookManager,
    reloader: ConfigReloader,
    reload_triggers: ReloadTriggers,
    resolved: Arc<[ComponentDescriptor]>,
    singletons: Vec<ComponentDescriptor>,
    scope_registry: Arc<ScopeRegistry>,
    scope_orders: Arc<HashMap<&'static str, Vec<ComponentDescriptor>>>,
    resolvers: ResolverSet,
}

impl<P: ProtocolPlugin> AppBuilder<P> {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registry: AppRegistry::default(),
            instances: Vec::new(),
            config_source: None,
            auto_discover_configs: false,
            dirs: None,
            protocol: P::default(),
        }
    }

    /// Mutable access to the protocol accumulator, for protocol-specific extension traits.
    pub fn protocol_mut(&mut self) -> &mut P {
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
    /// protocol's own variants, via [`Plugin::auto_discover`]) into the app, and enables
    /// config auto-discovery.
    pub fn auto_discover(mut self) -> Self {
        let discovered = AppRegistry::collect();

        self.registry.components.extend(discovered.components);
        self.registry.providers.extend(discovered.providers);
        self.auto_discover_configs = true;
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

    /// Applies a non-protocol [`Plugin`], folding its registrations into the app.
    pub fn plugin<Q: Plugin>(mut self, plugin: Q) -> Self {
        plugin.register(&mut self.registry);

        self
    }

    /// Registers and validates the application without constructing ordinary components.
    pub fn prepare(self) -> Result<PreparedApp<P>, P::Error> {
        debug!(target: "overseerd::app", app = %self.name, "building app");

        let mut registry = self.registry;
        let mut instances = self.instances;
        let mut protocol = self.protocol;

        // Consumed by `serve`/`run`; its handle is seeded as a framework injectable.
        let shutdown = ShutdownSignal::new();

        // A run-time handle to the finished root container, seeded now (empty) and attached
        // once the root is built, so a singleton can resolve from the container after startup.
        let root_resolver = RootResolver::new();

        // The protocol plugin contributes its DI descriptors (for RPC, the connection-scoped
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

        // Finalize the config manager. Auto-discovery (gated on the builder's `auto_discover`)
        // registers `#[config(path)]` types and seeds defaults; explicit bindings fold in next.
        let explicit_bindings = std::mem::take(&mut registry.config_bindings);
        let mut tree = match self.config_source {
            Some(config) => config,
            None => ConfigManager::load_in(&dirs.config_path(), &[]).map_err(Error::from)?,
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

        let scopes = ScopePlan::partition(&resolved, &registry.providers, P::SCOPES)?;

        // Build the config store — every bound `Cfg<T>` value, plus the reload slots.
        let (config_store, reload_slots) = ConfigStore::build(&tree).map_err(Error::from)?;
        let config_store = Arc::new(config_store);

        protocol.pre_build(&PreBuildContext::new(
            &self.name,
            &registry,
            config_store.as_ref(),
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
            shutdown,
            root_resolver,
            hook_manager,
            reloader,
            reload_triggers,
            resolved: Arc::from(resolved),
            singletons: scopes.singletons,
            scope_registry,
            scope_orders: Arc::new(scopes.orders),
            resolvers,
        })
    }

    /// Validates, constructs, and finalizes a ready-to-run [`App`].
    pub async fn build(self) -> Result<App<P>, P::Error> {
        self.prepare()?.build().await
    }
}

impl<P: ProtocolPlugin> PreparedApp<P> {
    /// The configured application name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The validated effective application registry.
    pub fn registry(&self) -> &AppRegistry {
        &self.registry
    }

    /// The validated protocol accumulator awaiting runtime construction.
    pub fn protocol(&self) -> &P {
        &self.protocol
    }

    /// Constructs ordinary components and finalizes the application protocol.
    pub async fn build(self) -> Result<App<P>, P::Error> {
        let PreparedApp {
            name,
            registry,
            instances,
            protocol,
            shutdown,
            root_resolver,
            hook_manager,
            reloader,
            reload_triggers,
            resolved,
            singletons,
            scope_registry,
            scope_orders,
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
            scope_orders,
            resolved,
            hook_manager,
        );

        // Hand off to the protocol plugin: it finalizes the served protocol.
        let protocol = protocol.build(&runtime)?;

        Ok(App {
            name,
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

/// The per-scope construction plan computed once at app build.
///
/// Agnostic: the intermediate scopes come from the protocol's declared chain (`P::SCOPES`),
/// and the per-scope construction `orders` are keyed by scope name.
struct ScopePlan {
    singletons: Vec<ComponentDescriptor>,
    transient: std::collections::HashMap<TypeId, ComponentDescriptor>,
    orders: std::collections::HashMap<&'static str, Vec<ComponentDescriptor>>,
}

impl ScopePlan {
    /// Splits the resolved components by scope and precomputes the construction order for
    /// each scope in the protocol's chain. A factory-less scoped component (e.g. a seeded
    /// peer) is treated as prebuilt. A constructable component whose scope is not in the
    /// chain (nor a singleton/transient) is a build error.
    fn partition(
        resolved: &[ComponentDescriptor],
        providers: &[ProviderDescriptor],
        scopes: &[&'static dyn Scope],
    ) -> crate::Result<Self> {
        let singleton_rank = SingletonScope.rank();

        let mut singletons = Vec::new();
        let mut transient = std::collections::HashMap::new();
        let mut by_scope: std::collections::HashMap<&'static str, Vec<ComponentDescriptor>> =
            std::collections::HashMap::new();
        let mut prebuilt: HashSet<TypeId> = HashSet::new();

        for c in resolved {
            if c.scope.is_transient() {
                transient.insert(c.ty.type_id, *c);
            } else if c.scope.rank() == singleton_rank {
                singletons.push(*c);
            } else if c.effective_factory()?.is_some() {
                by_scope.entry(c.scope.name()).or_default().push(*c);
            } else {
                prebuilt.insert(c.ty.type_id);
            }
        }

        prebuilt.extend(singletons.iter().map(|c| c.ty.type_id));

        let mut orders = std::collections::HashMap::new();

        for scope in scopes {
            let components = by_scope.remove(scope.name()).unwrap_or_default();
            let order: Vec<ComponentDescriptor> =
                topological_sort(&components, &prebuilt, providers, &transient)?
                    .into_iter()
                    .copied()
                    .collect();

            prebuilt.extend(order.iter().map(|c| c.ty.type_id));
            orders.insert(scope.name(), order);
        }

        if let Some((scope, components)) = by_scope.iter().next() {
            return Err(crate::Error::UndeclaredScope {
                component: components[0].name.to_string(),
                scope,
            });
        }

        Ok(Self {
            singletons,
            transient,
            orders,
        })
    }
}

/// A fully assembled app, ready to serve its protocol.
///
/// Holds the agnostic [`AppRuntime`] (DI container, scope orders, hooks) and the built
/// [`Protocol`], plus the shutdown signal and config reloader the serve envelope drives.
pub struct App<P: ProtocolPlugin> {
    pub name: String,
    pub registry: AppRegistry,
    runtime: AppRuntime,
    protocol: P::Protocol,
    shutdown: ShutdownSignal,
    reloader: ConfigReloader,
    reload_triggers: ReloadTriggers,
}

impl<P: ProtocolPlugin> fmt::Debug for App<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("App")
            .field("name", &self.name)
            .field("components", &self.registry.components.len())
            .finish_non_exhaustive()
    }
}

impl<P: ProtocolPlugin> fmt::Display for App<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "App: {}", self.name)?;
        write!(f, "{}", self.registry)?;

        Ok(())
    }
}

impl<P: ProtocolPlugin> App<P> {
    /// Starts building an app for protocol plugin `P`. Most protocols expose a pinned
    /// alias (e.g. `overseerd_rpc::App = App<RpcPlugin>`) so `App::builder(name)` resolves
    /// without a turbofish.
    pub fn builder(name: impl Into<String>) -> AppBuilder<P> {
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
    pub fn protocol(&self) -> &P::Protocol {
        &self.protocol
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
    pub async fn serve<E>(self, endpoint: E) -> Result<(), <P::Protocol as Protocol>::Error>
    where
        P::Protocol: Serve<E>,
        <P::Protocol as Protocol>::Error: From<crate::Error>,
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
