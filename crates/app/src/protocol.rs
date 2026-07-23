//! The pluggable protocol seam (traits).
//!
//! These traits are protocol-agnostic; the native RPC protocol implements them in the
//! `overseerd-rpc` crate, a future axum protocol in its own crate.

use std::future::Future;

use overseerd_config::{Cfg, ConfigProperties, ConfigStore};
use overseerd_core::Scope;
use overseerd_di::ComponentDescriptor;

use crate::lifecycle::ShutdownSignal;
use crate::registry::AppRegistry;
use crate::runtime::AppRuntime;

/// A general extension unit applied to an app while it is built.
///
/// A plugin is the builder-time accumulator for an extension: it starts empty
/// ([`Default`]), gathers protocol-specific configuration through the builder, folds its
/// link-time-discovered variants in on `auto_discover`, and contributes DI descriptors
/// into the registry before the container is built. A plugin need not serve traffic —
/// that is the job of the [`ProtocolPlugin`] sub-trait. Background behavior rides the
/// components a plugin registers (via their own `#[hook]`s).
pub trait Plugin: Default {
    /// Folds this plugin's link-time-registered component variants (e.g. the RPC
    /// `SERVICES` slice) into the accumulator. Called from `AppBuilder::auto_discover`.
    /// Default: nothing to discover.
    fn auto_discover(&mut self) {}

    /// Contributes DI descriptors / seeds into the registry before validation and build.
    /// The native RPC plugin seeds its connection-scoped `PeerInfo` here.
    fn register(&self, registry: &mut AppRegistry);
}

/// A [`Plugin`] that additionally installs a serve/dispatch [`Protocol`]. An `App` is
/// built around exactly one of these.
pub trait ProtocolPlugin: Plugin {
    /// The protocol this plugin installs.
    type Protocol: Protocol;
    /// The plugin's error type; must absorb agnostic build failures.
    type Error: std::error::Error + Send + Sync + 'static + From<crate::Error>;

    /// The session scope chain this protocol opens, root→leaf by rank, *excluding* the
    /// universal `Singleton` (root) and `Transient` (per-resolve). RPC opens
    /// `[Connection, Request]`; a request-only protocol opens `[Request]`.
    const SCOPES: &'static [&'static dyn Scope];

    /// Validates protocol-owned configuration and descriptors before ordinary components are
    /// constructed. Implementations may cache derived metadata for [`build`](Self::build).
    fn pre_build(&mut self, context: &PreBuildContext<'_>) -> Result<(), Self::Error> {
        let _ = context;

        Ok(())
    }

    /// Finalizes the protocol from the accumulated builder state and the assembled
    /// runtime — for RPC, building the router from the discovered services and folding
    /// the middleware stack.
    fn build(self, runtime: &AppRuntime) -> Result<Self::Protocol, Self::Error>;
}

/// Read-only application state available during protocol pre-build validation.
pub struct PreBuildContext<'a> {
    name: &'a str,
    registry: &'a AppRegistry,
    config: &'a ConfigStore,
}

impl<'a> PreBuildContext<'a> {
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

    /// Resolves a prepared configuration binding by type and property path.
    pub fn config<T: ConfigProperties>(&self, path: &str) -> Option<Cfg<T>> {
        self.config.resolve_path::<Cfg<T>>(path)
    }
}

/// A pluggable serve/dispatch layer over the app's DI runtime. There is exactly one
/// active protocol per `App`.
pub trait Protocol: Send + 'static {
    type Error: std::error::Error + Send + Sync + 'static;
}

/// Serves a built protocol over a concrete endpoint type `E`. Kept separate from
/// [`Protocol`] so one protocol can serve many endpoint types — RPC over any transport, a
/// future HTTP protocol over a `SocketAddr`. The serve loop only needs to watch `endpoint`
/// and `shutdown`; lifecycle and reload are handled by the caller (`App::serve`).
pub trait Serve<E>: Protocol {
    fn serve(
        self,
        runtime: AppRuntime,
        shutdown: ShutdownSignal,
        endpoint: E,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
