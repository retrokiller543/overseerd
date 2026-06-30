//! The pluggable protocol seam (traits).
//!
//! These traits are protocol-agnostic; the native RPC protocol implements them in the
//! `overseerd-rpc` crate, a future axum protocol in its own crate.

use std::future::Future;

use overseerd_core::Scope;

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

    /// Finalizes the protocol from the accumulated builder state and the assembled
    /// runtime — for RPC, building the router from the discovered services and folding
    /// the middleware stack.
    fn build(self, runtime: &AppRuntime) -> Result<Self::Protocol, Self::Error>;
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
