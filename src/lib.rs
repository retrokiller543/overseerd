//! # Overseerd
//!
//! A component- and service-oriented framework. Depend on this single crate; it re-exports
//! the layered core — `overseerd-core` (vocabulary + resolver), `overseerd-di` (the DI
//! engine), `overseerd-hooks`, `overseerd-dirs`, `overseerd-config`, and `overseerd-app`
//! (the protocol-agnostic application core) — plus the procedural macros.
//!
//! The native RPC daemon lives in its own `overseerd-rpc` crate and is re-exported here
//! behind the default-on **`daemon`** feature (disable it with `default-features = false`
//! for a core-framework-only build, or depend on `overseerd-rpc` directly).

// ---------------------------------------------------------------------------
// Leaf vocabulary + resolver model.
// ---------------------------------------------------------------------------
#[cfg(not(target_family = "wasm"))]
pub use overseerd_core::*;

/// Component lifetime scopes: the [`Scope`] trait a protocol's scope chain is built from,
/// plus the marker types. A component selects one with `#[component(scope = Request)]`; a
/// `#[component]` defaults to [`Singleton`](scope::Singleton).
///
/// The core defines only the universal anchors [`Singleton`](scope::Singleton) and
/// [`Transient`](scope::Transient); `Connection` and `Request` are RPC-protocol scopes from
/// `overseerd-rpc`, available with the `daemon` feature.
#[cfg(not(target_family = "wasm"))]
pub mod scope {
    pub use overseerd_core::scope::{Singleton, Transient};

    #[cfg(feature = "daemon")]
    pub use overseerd_rpc::scope::{Connection, Request}; // should not be here, should be in crate::daemon with the rest of the daemon related items
}

// ---------------------------------------------------------------------------
// DI engine: descriptors, container, factories, registry.
// ---------------------------------------------------------------------------
#[cfg(not(target_family = "wasm"))]
pub use overseerd_di::{
    BoxedComponent, COMPONENTS, Component, ComponentConstructionContext, ComponentContainer,
    ComponentDescriptor, ComponentFactories, ComponentFactory, ComponentFactoryDescriptor,
    ComponentRegistry, ComponentSource, Deferred, Dep, Dynamic, Factory, FactoryOutput, Fresh,
    FreshFromContainer, FromContainer, Injectable, Lazy, Live, LiveRef, PROVIDERS, Provide,
    ProviderDescriptor, ProviderOf, ProviderOrder, ProviderOrderDirection, RootResolver,
    ScopeContainer, ServiceComponent, Wired, Wiring, dispatch_factory, factory_dependencies,
    from_boxed,
};
/// The DI layer's own error/result, exposed under distinct names so macro-generated
/// **factory** code can name them without colliding with the root [`Error`]/[`Result`].
#[cfg(not(target_family = "wasm"))]
pub use overseerd_di::{Error as DiError, Result as DiResult};

// ---------------------------------------------------------------------------
// Hooks.
// ---------------------------------------------------------------------------
#[cfg(not(target_family = "wasm"))]
pub use overseerd_hooks::{
    ComponentHooks, HookCall, HookDescriptor, HookKind, HookManager, HookParam, Shutdown, Startup,
    no_hooks,
};
/// The hook layer's own error/result, exposed under distinct names so macro-generated hook
/// code can name them without colliding with the root [`Error`]/[`Result`].
#[cfg(not(target_family = "wasm"))]
pub use overseerd_hooks::{Error as HookError, Result as HookResult};

// ---------------------------------------------------------------------------
// Directories.
// ---------------------------------------------------------------------------
#[cfg(not(target_family = "wasm"))]
pub use overseerd_dirs::{Dir, DirKind, DirectoriesManager};

// ---------------------------------------------------------------------------
// Config: Cfg, ConfigManager, reload, the config store, the directory resolver.
// ---------------------------------------------------------------------------
#[cfg(not(target_family = "wasm"))]
pub use overseerd_config::{
    CONFIG_BINDINGS, Cfg, CfgNext, ChangedBinding, ComponentHookReport, ConfigBinding,
    ConfigBindingDescriptor, ConfigDefaults, ConfigError, ConfigManager, ConfigProperties,
    ConfigReload, ConfigReloadError, ConfigReloadReport, ConfigReloader, ConfigStore,
    ContainerConfigExt, DefaultSpec, DirectoriesResolver, EnumTag, HookOutcome, ReloadProposal,
    ReloadTriggers, ReloadableConfig, spawn_reload_triggers, stop_reload_triggers,
};

// ---------------------------------------------------------------------------
// Protocol-agnostic application core (always available): the App/Plugin seam, the runtime
// handle, lifecycle/shutdown, and the opt-in config-property builtins.
// ---------------------------------------------------------------------------
#[cfg(not(target_family = "wasm"))]
pub use overseerd_app::{
    App, AppBuilder, AppRegistry, AppRuntime, LoggingConfig, Plugin, Protocol, ProtocolPlugin,
    Serve, ServerConfig, ShutdownHandle, ShutdownSignal,
};

// The generic `App<P>` / `AppBuilder<P>` are at the root (protocol-agnostic core); the `app!`
// macro builds `App::<P>::builder(..)` for the protocol named in its `protocol:` field. A
// protocol's own surface (the RPC daemon's services, client, …) lives in its module
// (`overseerd::daemon::*`), so the facade root stays free of plugin-specific names.

// ---------------------------------------------------------------------------
// Wire-contract status types and stream item codecs.
// ---------------------------------------------------------------------------
pub use overseerd_transport::{
    Flags, PredefinedCode, StatusCode, StreamDecode, StreamDecodeError, StreamEncode,
    StreamEncodeError,
};

// ---------------------------------------------------------------------------
// Procedural macros: the core macros (including the protocol-agnostic `app!`/`daemon!`) are
// always available; the RPC daemon macros (`service`/`handlers`/`rpc`) live in the `daemon`
// module behind the `daemon` feature.
// ---------------------------------------------------------------------------
pub use overseerd_macros::{app, component, config, daemon, injectable, methods};

/// Re-exported so macro-generated code can reference the `#[distributed_slice]` attribute
/// through the facade crate without user crates depending on `linkme` directly. The generated
/// registration that names it is server-only (gated out on wasm), so this is too.
#[cfg(not(target_family = "wasm"))]
#[doc(hidden)]
pub use overseerd_di::linkme;

// ---------------------------------------------------------------------------
// Transport substrate: server endpoints, wire types, custom-transport traits. The RPC
// *client* lives under `daemon` (it is protocol-specific), not here.
// ---------------------------------------------------------------------------
#[cfg(not(target_family = "wasm"))]
pub use overseerd_transport::{
    CallId, CallResult, Connection, IncomingCall, MemoryCall, MemoryClient, MemoryConnection,
    MemoryConnectionHandle, MemoryResponder, MemoryTransport, PeerInfo, Respond, RespondStream,
    ResponseSink, ServerEvent, TcpTransport, Transport, WireMessage, WireOutcome, WireRequest,
    WireResponse,
};

#[cfg(unix)]
pub use overseerd_transport::UnixTransport;

/// The full transport substrate, including the framing codec and connection/responder
/// types for building custom transports.
pub mod transport {
    pub use overseerd_transport::*;
}

/// The protocol-agnostic **client** core: the [`ProtocolTransport`](client::ProtocolTransport)
/// abstraction and the typed [`Client`](client::Client) surface generated clients build on.
/// Always available (like [`app`]-layer items); a protocol (the RPC `daemon`, …) supplies a
/// `ProtocolTransport` impl. Generated client code roots here, so it is identical across
/// protocols.
pub mod client {
    pub use overseerd_client::*;
}

/// Application directory kinds (`Config`, `Data`, `Cache`, `State`, `Runtime`, `Tmp`), the
/// typed [`Dir`] wrapper, and the [`DirectoriesManager`]. Inject `Dir<dirs::Config>`.
#[cfg(not(target_family = "wasm"))]
pub mod dirs {
    pub use overseerd_dirs::*;
}

/// Config source-format markers for `ConfigManager<F>`: [`Toml`](overseerd_config::Toml),
/// the format-erased [`Dynamic`](overseerd_config::Dynamic), and (with the `yaml` feature)
/// `Yaml`.
#[cfg(not(target_family = "wasm"))]
pub mod config {
    pub use overseerd_config::{Dynamic, Format, FormatId, Toml};

    #[cfg(feature = "yaml")]
    pub use overseerd_config::Yaml;
}

/// Framework builtins: the seeded [`ShutdownHandle`] injectable, the opt-in
/// [`ServerConfig`] / [`LoggingConfig`] property structs, and the feature-gated
/// `init_tracing` subscriber helper.
#[cfg(not(target_family = "wasm"))]
pub mod builtins {
    pub use overseerd_app::builtins::{LoggingConfig, ServerConfig};

    #[cfg(feature = "tracing-subscriber")]
    pub use overseerd_app::builtins::{InitTracingError, init_tracing};
}

/// The native RPC **daemon** plugin surface, namespaced so plugin items never collide with
/// the facade root or with other plugins (`overseerd::axum::*`, …).
///
/// Build an RPC app with `use overseerd::prelude::*;` (the core framework) plus
/// `use overseerd::daemon::prelude::*;` (the common RPC items). The daemon macros
/// (`#[service]`/`#[handlers]`/`#[rpc]`/`app!`) emit `::overseerd::daemon::*` paths, so end
/// users depend only on `overseerd` — never on `overseerd-rpc` directly.
#[cfg(all(feature = "daemon", not(target_family = "wasm")))]
pub mod daemon {
    pub use overseerd_rpc::{
        App, AppBuilder, Cancel, Error, ErrorHandler, ErrorResponse, FallibleHandler, FromContext,
        Guard, GuardLayer, GuardService, Handler, Inject, OperationKind, ParameterDescriptor,
        ParameterKind, Payload, Peer, RequestStream, ResolvedService, Responder, ResponseError,
        ResponseStream, Result, RouterService, Rpc, RpcAppBuilder, RpcCallContext, RpcDescriptor,
        RpcGroup, RpcHandler, RpcLimits, RpcOutcome, RpcPlugin, RpcRequest, RpcResponse, RpcRouter,
        RpcService, SERVICES, ServiceDescriptor, ServiceRpcs, Streaming, dispatch_fallible,
        dispatch_with,
    };

    /// Service/RPC route resolution, for introspecting the registered surface.
    pub use overseerd_rpc::routes::resolved_services;

    /// The RPC component scopes.
    pub use overseerd_rpc::scope::{Connection, Request};

    /// The RPC daemon macros, re-exported through `overseerd-rpc` (which owns them). With the
    /// facade's `daemon` feature, `overseerd-rpc/facade` is on, so their generated code roots
    /// plugin types at `::overseerd::daemon::*` and core types at `::overseerd::*`.
    /// (`app!`/`daemon!` are protocol-agnostic core macros at the crate root, not here.)
    pub use overseerd_rpc::{handlers, rpc, service};

    /// Re-exported so middleware authors can implement `tower::Layer` / `tower::Service`.
    pub use overseerd_rpc::tower;

    /// Re-exported so `#[rpc(stream)]` client codegen can project a concrete stream's item
    /// type. Hidden; referenced only by generated code.
    #[doc(hidden)]
    pub use overseerd_rpc::__Stream;

    /// The RPC byte-stream transport — the daemon's [`ProtocolTransport`](crate::client) impl
    /// (`StreamClientTransport`), its response stream, and connect helpers. The agnostic client
    /// surface (`Client` API, capability traits) lives at [`overseerd::client`](crate::client);
    /// this is the RPC carry that plugs into it. Gated behind the `client` feature.
    #[cfg(feature = "client")]
    pub use overseerd_rpc::{RpcResponses, StreamClientTransport, connect_tcp};

    #[cfg(all(feature = "client", unix))]
    pub use overseerd_rpc::connect_unix;

    /// Deprecated alias for [`App`]. Renamed in 0.7.0; removed in 1.0.0.
    #[deprecated(since = "0.7.0", note = "renamed to `App`; removed in 1.0.0")]
    pub type Daemon = App;

    /// Deprecated alias for [`AppBuilder`]. Renamed in 0.7.0; removed in 1.0.0.
    #[deprecated(since = "0.7.0", note = "renamed to `AppBuilder`; removed in 1.0.0")]
    pub type DaemonBuilder = AppBuilder;

    /// Common imports for building an RPC daemon: `use overseerd::daemon::prelude::*;` (pair
    /// with the crate-root `use overseerd::prelude::*;` for the core framework + `app!`).
    pub mod prelude {
        pub use super::{
            App, FromContext, Handler, Inject, Payload, Peer, Responder, RpcAppBuilder, RpcPlugin,
            Streaming, handlers, rpc, service,
        };

        pub use overseerd_transport::TcpTransport;

        #[cfg(unix)]
        pub use overseerd_transport::UnixTransport;
    }
}

/// The job scheduler: run `async` methods on an interval or cron schedule as supervised
/// background tasks, or schedule work at run time.
///
/// A protocol-agnostic [`Plugin`](crate::Plugin) — enable the `jobs` feature and register
/// [`JobsPlugin`](jobs::JobsPlugin) alongside any protocol. Mark methods with
/// `#[job(every = "..")]` / `#[job(cron = "..")]` (the `#[job]` codegen emits
/// `::overseerd::jobs::*` paths), or inject `Arc<JobScheduler>` and call
/// [`schedule`](jobs::JobScheduler::schedule) for dynamic jobs.
#[cfg(all(feature = "jobs", not(target_family = "wasm")))]
pub mod jobs {
    pub use overseerd_jobs::*;
}

/// The axum/HTTP protocol surface, namespaced so plugin items never collide with the facade
/// root or with the RPC `daemon` module.
///
/// Build an HTTP app with `use overseerd::prelude::*;` (the core framework + `app!`) plus
/// `use overseerd::axum::prelude::*;` (controllers, route attributes, the DI `Inject`
/// extractor, and the common axum extractors). The controller macros
/// (`#[controller]`/`#[handlers]`/`#[get]`/…) emit `::overseerd::axum::*` paths, so end users
/// depend only on `overseerd` — never on `overseerd-axum` directly.
#[cfg(feature = "axum")]
pub mod axum {
    /// The whole `overseerd-axum` surface, glob-re-exported so the facade never drifts behind the
    /// protocol crate: the controller types/macros, the framing wrappers, the `axum` crate and
    /// `http` re-exports, the `scope` module, and (with the `client` feature) the `client` module
    /// and `__Stream` the generated client names. The protocol crate's root *is* the curated API.
    /// With the facade's `axum` feature, `overseerd-axum/facade` is on, so the macros' generated
    /// code roots plugin types at `::overseerd::axum::*` and core types at `::overseerd::*`.
    pub use overseerd_axum::*;
    #[cfg(feature = "json-ws")]
    pub use overseerd_axum_json_ws::*;
    #[cfg(feature = "stomp")]
    pub use overseerd_axum_stomp::*;

    #[cfg(all(feature = "stomp", feature = "client"))]
    pub mod stomp_client {
        pub use overseerd_axum_stomp::StompStatus;

        #[cfg(feature = "tungstenite")]
        pub use overseerd_axum_stomp::{StompClientTransport, StompConnectOptions};
    }

    /// Common imports for building an HTTP controller app: `use overseerd::axum::prelude::*;`
    /// (pair with the crate-root `use overseerd::prelude::*;` for the core framework + `app!`).
    ///
    /// The controller/route macros are available on every target; the server extractors, `Router`,
    /// `App`, and DI types are the server surface (compiled out on a wasm client build, where the
    /// generated `{Controller}Client` is used directly rather than the prelude).
    pub mod prelude {
        pub use super::{
            controller, delete, dto, get, handlers, head, options, patch, post, put, route,
        };

        #[cfg(not(target_family = "wasm"))]
        pub use super::axum::extract::{Json, Path, Query, State};
        #[cfg(not(target_family = "wasm"))]
        pub use super::axum::response::IntoResponse;
        #[cfg(not(target_family = "wasm"))]
        pub use super::axum::{Router, http};
        #[cfg(not(target_family = "wasm"))]
        pub use super::scope::Request;
        #[cfg(not(target_family = "wasm"))]
        pub use super::{
            App, AxumAppBuilder, AxumAppServe, AxumConfig, AxumPlugin, Controller, Inject,
        };

        #[cfg(all(feature = "json-ws", not(target_family = "wasm")))]
        pub use super::JsonWs;
        /// WebSocket controller imports (`#[controller(ws = ..)]` + `#[message]`, the per-connection
        /// [`Connection`](super::scope::Connection) scope), available with the `ws` feature.
        #[cfg(all(feature = "ws", not(target_family = "wasm")))]
        pub use super::scope::Connection;
        #[cfg(all(feature = "ws", not(target_family = "wasm")))]
        pub use super::{WebsocketController, WebsocketProtocol, message};

        /// STOMP pub/sub imports (`#[controller(ws = Stomp)]` + `#[topics]`, the typed
        /// [`Publisher`](super::Publisher) and [`Topic`](super::Topic)), available with the
        /// `stomp` feature.
        #[cfg(all(feature = "stomp", not(target_family = "wasm")))]
        pub use super::{
            Publisher, Stomp, StompConfig, StompConnect, StompPrincipal, StompSession,
            StompTopicBus, Topic, topics,
        };
    }
}

/// The common imports for the **core framework**: `use overseerd::prelude::*;`. Pair with
/// `use overseerd::daemon::prelude::*;` (or another plugin's prelude) for the protocol layer.
pub mod prelude {
    // The macros are built for the host and usable from any target (their generated server code is
    // gated out on wasm); the framework *types* are the daemon-stack surface, compiled out on wasm.
    pub use crate::{app, component, config, injectable, methods};

    #[cfg(not(target_family = "wasm"))]
    pub use crate::{
        App, Cfg, Component, ConfigManager, ConfigProperties, Deferred, Dep, Dir, DirKind,
        DirectoriesManager, Fresh, FreshFromContainer, Injectable, Lazy, Plugin, Protocol,
        ProtocolPlugin, RuntimeDescriptor, Scope, Serve, ServiceComponent,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_exposes_core_types() {
        let td = TypeDescriptor::of::<u8>("byte");

        assert_eq!(td.name, "byte");
        assert_eq!(td.type_id, type_id_of::<u8>());
    }
}
