//! # Overseerd
//!
//! A component- and service-oriented RPC framework. Depend on this single crate; it
//! re-exports everything from the layered implementation crates — `overseerd-core`
//! (vocabulary + resolver), `overseerd-di` (the DI engine), `overseerd-hooks`,
//! `overseerd-dirs`, `overseerd-config`, `overseerd-daemon` (runtime + RPC), and
//! `overseerd-transport` — plus the procedural macros.
//!
//! ## Concepts
//!
//! - **Component** — a singleton dependency. Either *system-constructed* (declared
//!   with [`component`] / a stateful [`service`], built by the container from its
//!   dependencies) or *manually provided* ([`AppBuilder::with_component`]).
//! - **Service** — a [`service`] type whose [`handlers`] impls expose `#[rpc]`
//!   methods. A stateful service is also a component (its `&self` is the singleton).
//! - **Container** — holds the constructed instances ([`ComponentContainer`]).
//! - **Registry** — holds the *declarations* ([`DescriptorRegistry`]).

// ---------------------------------------------------------------------------
// Leaf vocabulary + resolver model.
// ---------------------------------------------------------------------------
pub use overseerd_core::{
    Cardinality, DependencyDescriptor, Descriptor, Resolver, ResolverCtx, ResolverCtxExt,
    ResolverSet, Scope, StaticScope, TypeDescriptor, type_id_of,
};

/// Component lifetime scopes: the [`Scope`] trait a protocol's scope chain is built
/// from, plus the marker types. A component selects one with `#[component(scope = Request)]`;
/// a `#[component]` defaults to [`Singleton`](scope::Singleton).
///
/// The core defines only the universal anchors [`Singleton`](scope::Singleton) and
/// [`Transient`](scope::Transient); [`Connection`](scope::Connection) and
/// [`Request`](scope::Request) are RPC-protocol scopes from `overseerd-daemon`. Kept in
/// a module so the common names don't collide with the transport [`Connection`] trait at
/// the crate root.
pub mod scope {
    pub use overseerd_core::scope::{Singleton, Transient};
    pub use overseerd_daemon::scope::{Connection, Request};
}

// ---------------------------------------------------------------------------
// DI engine: descriptors, container, factories, registry.
// ---------------------------------------------------------------------------
pub use overseerd_di::{
    BoxedComponent, COMPONENTS, Component, ComponentConstructionContext, ComponentContainer,
    ComponentDescriptor, ComponentFactories, ComponentFactory, ComponentFactoryDescriptor,
    ComponentRegistry, ComponentSource, Dep, Dynamic, Factory, FactoryOutput, FromContainer,
    Injectable, Live, LiveRef, PROVIDERS, Provide, ProviderDescriptor, ScopeContainer,
    ServiceComponent, Wired, Wiring, dispatch_factory, factory_dependencies, from_boxed,
};
/// The DI layer's own error/result, exposed under distinct names so macro-generated
/// **factory** code (which constructs components) can name them without colliding with the
/// daemon's root [`Error`]/[`Result`] (used by RPC handler codegen).
pub use overseerd_di::{Error as DiError, Result as DiResult};

// ---------------------------------------------------------------------------
// Hooks.
// ---------------------------------------------------------------------------
pub use overseerd_hooks::{
    ComponentHooks, HookCall, HookDescriptor, HookKind, HookManager, HookParam, Shutdown, Startup,
    no_hooks,
};
/// The hook layer's own error/result, exposed under distinct names so macro-generated hook
/// code can name them without colliding with the daemon's root [`Error`]/[`Result`].
pub use overseerd_hooks::{Error as HookError, Result as HookResult};

// ---------------------------------------------------------------------------
// Directories.
// ---------------------------------------------------------------------------
pub use overseerd_dirs::{Dir, DirKind, DirectoriesManager};

// ---------------------------------------------------------------------------
// Config: Cfg, ConfigManager, reload, the config store, the directory resolver.
// ---------------------------------------------------------------------------
pub use overseerd_config::{
    CONFIG_BINDINGS, Cfg, CfgNext, ChangedBinding, ComponentHookReport, ConfigBinding,
    ConfigBindingDescriptor, ConfigDefaults, ConfigError, ConfigManager, ConfigProperties,
    ConfigReload, ConfigReloadError, ConfigReloadReport, ConfigReloader, ConfigStore,
    ContainerConfigExt, DefaultSpec, DirectoriesResolver, EnumTag, HookOutcome, ReloadProposal,
    ReloadTriggers, ReloadableConfig, spawn_reload_triggers,
};

// ---------------------------------------------------------------------------
// App runtime + RPC: builder, router, extractors, middleware, registry, errors.
// ---------------------------------------------------------------------------
pub use overseerd_daemon::{
    App, AppBuilder, AppRuntime, Cancel, DescriptorRegistry, Error, ErrorHandler, ErrorResponse,
    FallibleHandler, FromContext, Guard, GuardLayer, GuardService, Handler, Inject, LoggingConfig,
    OperationKind, ParameterDescriptor, ParameterKind, Payload, Peer, Protocol, RequestStream,
    ResolvedService, Responder, ResponseError, ResponseStream, Result, RouterService, Rpc,
    RpcCallContext, RpcDescriptor, RpcGroup, RpcHandler, RpcOutcome, RpcRequest, RpcResponse,
    RpcRouter, RpcService, SERVICES, Serve, ServerConfig, ServiceDescriptor, ServiceRpcs,
    ShutdownHandle, ShutdownSignal, Streaming, dispatch_fallible, dispatch_with,
};

/// Deprecated alias for [`App`]. Renamed in 0.7.0; the alias is removed in 1.0.0.
#[deprecated(
    since = "0.7.0",
    note = "renamed to `App`; the `Daemon` alias is removed in 1.0.0"
)]
pub type Daemon = App;

/// Deprecated alias for [`AppBuilder`]. Renamed in 0.7.0; the alias is removed in 1.0.0.
#[deprecated(
    since = "0.7.0",
    note = "renamed to `AppBuilder`; the `DaemonBuilder` alias is removed in 1.0.0"
)]
pub type DaemonBuilder = AppBuilder;

// ---------------------------------------------------------------------------
// Wire-contract status types and stream item codecs.
// ---------------------------------------------------------------------------
pub use overseerd_transport::{
    Flags, PredefinedCode, StatusCode, StreamDecode, StreamDecodeError, StreamEncode,
    StreamEncodeError,
};

// ---------------------------------------------------------------------------
// Procedural macros.
// ---------------------------------------------------------------------------
pub use overseerd_macros::{
    app, component, config, daemon, handlers, injectable, methods, rpc, service,
};

/// Re-exported so macro-generated code can reference the `#[distributed_slice]`
/// attribute through the facade crate without user crates depending on `linkme`
/// directly.
#[doc(hidden)]
pub use overseerd_di::linkme;

/// Re-exported so middleware authors can implement `tower::Layer` / `tower::Service`
/// (and reach tower's own layers) without depending on `tower` directly.
pub use overseerd_daemon::tower;

/// Re-exported so generated client traits can be annotated `#[async_trait]`
/// (for `dyn`-compatibility) without user crates depending on `async-trait`.
#[cfg(feature = "client")]
#[doc(hidden)]
pub use async_trait;

/// Re-exported so a `#[rpc(stream)]` handler returning a concrete (un-introspectable)
/// stream type still yields a well-typed client: the generated code projects the
/// wire item type as `<ReturnType as Stream>::Item` through this alias.
#[doc(hidden)]
pub use futures::Stream as __Stream;

// ---------------------------------------------------------------------------
// Transport: server endpoints, client wire protocol, custom-transport traits.
// `Error`/`Result` are intentionally not lifted to the root (the daemon ones win);
// reach transport's via `overseerd::transport`.
// ---------------------------------------------------------------------------
pub use overseerd_transport::{
    CallId, CallResult, Connection, IncomingCall, MemoryCall, MemoryClient, MemoryConnection,
    MemoryConnectionHandle, MemoryResponder, MemoryTransport, PeerInfo, Respond, RespondStream,
    ResponseSink, ServerEvent, TcpTransport, Transport, WireMessage, WireOutcome, WireRequest,
    WireResponse,
};

#[cfg(unix)]
pub use overseerd_transport::UnixTransport;

/// Client SDK runtime: the substrate-agnostic [`ClientTransport`] abstraction,
/// the byte-stream implementation, and the typed [`ClientConnection`] the
/// generated clients build on. Gated behind the `client` feature.
#[cfg(feature = "client")]
pub use overseerd_transport::{
    BidiResponses, CallSink, CallSource, ClientCall, ClientConnection, ClientError,
    ClientTransport, ErrorBody, Raw, Reply, ServerStream, StreamArg, StreamCall, StreamCallSink,
    StreamClientTransport, StreamSource,
};

/// The full transport layer, including the framing codec (`transport::protocol::codec`)
/// and connection/responder types for building clients or custom transports.
pub mod transport {
    pub use overseerd_transport::*;
}

/// Application directory kinds (`Config`, `Data`, `Cache`, `State`, `Runtime`,
/// `Tmp`), the typed [`Dir`] wrapper, and the [`DirectoriesManager`] that resolves
/// them. Inject `Dir<dirs::Config>` and friends.
pub mod dirs {
    pub use overseerd_dirs::*;
}

/// Config source-format markers for `ConfigManager<F>`: [`Toml`](overseerd_config::Toml),
/// the format-erased [`Dynamic`](overseerd_config::Dynamic), and (with the `yaml` feature)
/// `Yaml`.
pub mod config {
    pub use overseerd_config::{Dynamic, Format, FormatId, Toml};

    #[cfg(feature = "yaml")]
    pub use overseerd_config::Yaml;
}

/// Framework builtins: the seeded [`ShutdownHandle`] injectable, the opt-in
/// [`ServerConfig`] / [`LoggingConfig`] property structs, and the feature-gated
/// `init_tracing` subscriber helper.
pub mod builtins {
    pub use overseerd_daemon::builtins::{LoggingConfig, ServerConfig};

    #[cfg(feature = "tracing-subscriber")]
    pub use overseerd_daemon::builtins::{InitTracingError, init_tracing};
}

/// The common imports for building a daemon: `use overseerd::prelude::*;`.
pub mod prelude {
    pub use crate::{
        App, Cfg, Component, ConfigManager, ConfigProperties, Handler, Inject, Payload, Result,
        ServiceComponent, component, handlers, rpc, service,
    };

    pub use overseerd_transport::TcpTransport;

    #[cfg(unix)]
    pub use overseerd_transport::UnixTransport;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_exposes_core_types() {
        let td = TypeDescriptor::of::<u8>("byte");

        assert_eq!(td.name, "byte");
        assert_eq!((td.type_id)(), (type_id_of::<u8>)());
    }
}
