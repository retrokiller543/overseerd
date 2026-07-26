//! Protocol definition, preparation, runtime, and plugin contracts.
//!
//! These traits are protocol-agnostic; the RPC and Axum protocol crates implement them.

use std::future::Future;

use overseerd_config::{Cfg, ConfigBinding, ConfigProperties, ConfigStore};
use overseerd_core::{Descriptor, TypeDescriptor};
use overseerd_di::{BoxedComponent, Component, ComponentDescriptor, Injectable};

use crate::ProtocolId;
use crate::lifecycle::ShutdownSignal;
use crate::registry::AppRegistry;
use crate::runtime::AppRuntime;
use crate::scope::ScopeTopology;

/// A general extension unit applied while an app definition is assembled.
///
/// A plugin is the builder-time accumulator for an extension: it starts empty
/// ([`Default`]), gathers extension-specific configuration through the builder, folds its
/// link-time-discovered variants in on `auto_discover`, and contributes DI descriptors
/// into the registry before the container is built. A plugin need not serve traffic.
/// Background behavior rides the components it registers through their own hooks.
pub trait Plugin: Default {
    /// Folds this plugin's link-time-registered component variants into the accumulator. Called
    /// from `AppBuilder::auto_discover`.
    /// Default: nothing to discover.
    fn auto_discover(&mut self) {}

    /// Contributes DI descriptors / seeds into the registry before validation and build.
    fn register(&self, registry: &mut AppRegistry);
}

/// A selected protocol definition before application validation and construction.
///
/// An application owns exactly one definition. It contributes protocol-owned registrations,
/// declares the scope topology, and is consumed into a distinct validated prepared state.
pub trait ProtocolDefinition: Default + 'static {
    /// The validated protocol state retained by [`PreparedApp`](crate::PreparedApp).
    type Prepared: PreparedProtocol<Error = Self::Error>;
    /// The definition's typed preparation and construction error.
    type Error: std::error::Error + Send + Sync + 'static + From<crate::Error>;

    /// Stable identity used by composition, diagnostics, and tooling.
    const ID: ProtocolId;

    /// Folds link-time discovered protocol descriptors into this definition.
    fn auto_discover(&mut self) {}

    /// Contributes protocol-owned descriptors before application validation.
    fn register(&self, registry: &mut AppRegistry);

    /// The protocol-owned scope boundaries and their declared parent paths.
    ///
    /// The universal singleton root is implicit, and transient components do not
    /// occupy an openable boundary.
    const SCOPE_TOPOLOGY: ScopeTopology;

    /// Contributes protocol-owned components and configuration bindings before app validation.
    fn pre_build(&mut self, context: &mut PreBuildContext<'_>) -> Result<(), Self::Error> {
        let _ = context;

        Ok(())
    }

    /// Validates finalized protocol-owned state and consumes the definition into its prepared
    /// representation without constructing runtime resources.
    fn prepare(self, context: &ValidationContext<'_>) -> Result<Self::Prepared, Self::Error>;
}

/// Validated protocol-specific state awaiting runtime construction.
pub trait PreparedProtocol: Send + 'static {
    /// The built protocol runtime.
    type Runtime: ProtocolRuntime;
    /// The typed construction error.
    type Error: std::error::Error + Send + Sync + 'static + From<crate::Error>;

    /// Builds the protocol runtime after the application's root DI container exists.
    fn build(self, runtime: &AppRuntime) -> Result<Self::Runtime, Self::Error>;
}

/// Mutable application state available for protocol contributions before validation.
pub struct PreBuildContext<'a> {
    registry: &'a mut AppRegistry,
    instances: &'a mut Vec<BoxedComponent>,
}

impl<'a> PreBuildContext<'a> {
    pub(crate) fn new(
        registry: &'a mut AppRegistry,
        instances: &'a mut Vec<BoxedComponent>,
    ) -> Self {
        Self {
            registry,
            instances,
        }
    }

    /// Registers a component descriptor for construction.
    pub fn component_descriptor(&mut self, descriptor: &ComponentDescriptor) {
        self.registry.components.push(*descriptor);
    }

    /// Registers component type `T` from its static descriptor.
    pub fn component<T>(&mut self)
    where
        T: Descriptor<ComponentDescriptor>,
    {
        self.registry
            .components
            .push(<T as Descriptor<ComponentDescriptor>>::DESCRIPTOR);
    }

    /// Registers a pre-built singleton component.
    pub fn with_component<T: Component>(&mut self, value: T) {
        self.registry
            .components
            .push(ComponentDescriptor::of::<T>());
        self.instances.push(BoxedComponent {
            ty: TypeDescriptor::of::<T>(T::NAME),
            value: Box::new(Injectable::into_stored(value.into_handle())),
        });
    }

    /// Binds configuration type `T` to `path` before config-store construction.
    pub fn config<T: ConfigProperties>(&mut self, path: impl Into<String>) {
        self.registry
            .config_bindings
            .push(ConfigBinding::of::<T>(path));
    }
}

/// Read-only finalized application state available for protocol validation.
pub struct ValidationContext<'a> {
    name: &'a str,
    registry: &'a AppRegistry,
    config: &'a ConfigStore,
}

impl<'a> ValidationContext<'a> {
    pub(crate) fn new(name: &'a str, registry: &'a AppRegistry, config: &'a ConfigStore) -> Self {
        Self {
            name,
            registry,
            config,
        }
    }

    /// The configured application name.
    pub fn name(&self) -> &str {
        self.name
    }

    /// The validated application registry.
    pub fn registry(&self) -> &AppRegistry {
        self.registry
    }

    /// The effective component descriptors selected during validation.
    pub fn resolved_components(&self) -> &[ComponentDescriptor] {
        &self.registry.components
    }

    /// Resolves a finalized configuration binding by type and property path.
    pub fn config<T: ConfigProperties>(&self, path: &str) -> Option<Cfg<T>> {
        self.config.resolve_path::<Cfg<T>>(path)
    }
}

/// A built serve/dispatch layer over the app's DI runtime.
pub trait ProtocolRuntime: Send + 'static {
    type Error: std::error::Error + Send + Sync + 'static;
}

/// Serves a built protocol over a concrete endpoint type `E`. Kept separate from
/// [`ProtocolRuntime`] so one protocol can serve many endpoint types — RPC over any transport, a
/// future HTTP protocol over a `SocketAddr`. The serve loop only needs to watch `endpoint`
/// and `shutdown`; lifecycle and reload are handled by the caller (`App::serve`).
pub trait Serve<E>: ProtocolRuntime {
    fn serve(
        self,
        runtime: AppRuntime,
        shutdown: ShutdownSignal,
        endpoint: E,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

impl Plugin for () {
    fn register(&self, _registry: &mut AppRegistry) {}
}

impl ProtocolDefinition for () {
    type Prepared = ();
    type Error = crate::Error;

    const ID: ProtocolId = overseerd_core::namespaced_id!(ProtocolId, "overseerd/none");
    const SCOPE_TOPOLOGY: ScopeTopology = ScopeTopology::empty();

    fn register(&self, _registry: &mut AppRegistry) {}

    fn prepare(self, _context: &ValidationContext<'_>) -> Result<Self::Prepared, Self::Error> {
        Ok(())
    }
}

impl PreparedProtocol for () {
    type Runtime = ();
    type Error = crate::Error;

    fn build(self, _runtime: &AppRuntime) -> Result<Self::Runtime, Self::Error> {
        Ok(())
    }
}

impl ProtocolRuntime for () {
    type Error = crate::Error;
}
