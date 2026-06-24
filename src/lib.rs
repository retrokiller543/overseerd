//! # Overseerd
//!
//! A component- and service-oriented RPC framework. Depend on this single crate;
//! it re-exports everything from the implementation crates (`overseerd-core` and
//! `overseerd-transport`) plus the procedural macros.
//!
//! ## Concepts
//!
//! - **Component** — a singleton dependency. Either *system-constructed* (declared
//!   with [`component`] / a stateful [`service`], built by the container from its
//!   dependencies) or *manually provided* ([`DaemonBuilder::with_component`]).
//! - **Service** — a [`service`] type whose [`handlers`] impls expose `#[rpc]`
//!   methods. A stateful service is also a component (its `&self` is the singleton).
//! - **Container** — holds the constructed instances ([`ComponentContainer`]).
//! - **Registry** — holds the *declarations* ([`DescriptorRegistry`]).
//!
//! ## Example
//!
//! ```ignore
//! use overseerd::prelude::*;
//! use serde::{Deserialize, Serialize};
//! use std::sync::Arc;
//!
//! #[derive(Component)]
//! struct Config { greeting: String }
//!
//! #[service(id = "greeter", version = "0.1")]
//! struct Greeter { config: Arc<Config> }
//!
//! #[derive(Deserialize)] struct GreetReq { name: String }
//! #[derive(Serialize)]   struct GreetResp { message: String }
//!
//! #[handlers]
//! impl Greeter {
//!     #[rpc]
//!     async fn greet(&self, Payload(req): Payload<GreetReq>) -> Result<GreetResp> {
//!         Ok(GreetResp { message: format!("{}, {}!", self.config.greeting, req.name) })
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let daemon = Daemon::builder("greeter")
//!         .auto_discover()
//!         .with_component(Config { greeting: "Hello".into() })
//!         .build()
//!         .await?;
//!
//!     daemon.serve(TcpTransport::bind("127.0.0.1:9000").await?).await
//! }
//! ```

// ---------------------------------------------------------------------------
// Core: descriptors, registry, container, daemon, extractors, macros.
// ---------------------------------------------------------------------------
pub use overseerd_core::{
    BoxedComponent, COMPONENTS, CONFIG_BINDINGS, Cancel, Cardinality, Cfg, Component,
    ComponentConstructionContext, ComponentContainer, ComponentDescriptor, ComponentFactories,
    ComponentFactory, ComponentFactoryDescriptor, ComponentScope, ConfigBinding,
    ConfigBindingDescriptor, ConfigError, ConfigManager,
    ConfigProperties, Daemon, DaemonBuilder, DependencyDescriptor, Descriptor, DescriptorRegistry,
    Factory, FactoryOutput, FromContainer, dispatch_factory, factory_dependencies,
    Dir, DirKind, DirectoriesManager, Dynamic, Error, ErrorHandler, ErrorResponse, FallibleHandler,
    Flags, FromContext, Guard, GuardLayer, GuardService, Handler, Inject, Injectable, LoggingConfig,
    OperationKind, PROVIDERS, ParameterDescriptor, ParameterKind, Payload, Peer, PredefinedCode,
    Provide, ProviderDescriptor, RequestStream, Responder, ResponseError,
    ResponseStream, Result, RouterService, RpcCallContext, RpcDescriptor, RpcGroup, RpcHandler,
    RpcOutcome, RpcRequest, RpcResponse, RpcRouter, RpcService, SERVICES, ScopeContainer,
    ServerConfig, ServiceComponent, ServiceDescriptor, ServiceRpcs, ShutdownHandle, ShutdownSignal,
    StatusCode, Streaming, TypeDescriptor, Wired, Wiring, component, daemon, dispatch_fallible,
    dispatch_with, handlers, injectable, methods, rpc, service, type_id_of,
};

/// The `#[config]` attribute macro. Re-exported straight from `overseerd_macros`
/// (where the name is unambiguously the macro) so it coexists with the [`config`]
/// module rather than colliding with it through `overseerd_core`'s dual-namespace
/// `config` name.
pub use overseerd_macros::config;

/// Re-exported so macro-generated code can reference the `#[distributed_slice]`
/// attribute through the facade crate without user crates depending on `linkme`
/// directly.
#[doc(hidden)]
pub use overseerd_core::linkme;

/// Re-exported so middleware authors can implement `tower::Layer` / `tower::Service`
/// (and reach tower's own layers) without depending on `tower` directly.
pub use overseerd_core::tower;

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
// `Error`/`Result` are intentionally not lifted to the root (the core ones win);
// reach transport's via `overseerd::transport`.
// ---------------------------------------------------------------------------
pub use overseerd_transport::{
    CallId, CallResult, Connection, IncomingCall, MemoryCall, MemoryClient, MemoryConnection,
    MemoryConnectionHandle, MemoryResponder, MemoryTransport, PeerInfo, Respond, RespondStream,
    ResponseSink, ServerEvent, StreamDecode, StreamDecodeError, StreamEncode, StreamEncodeError,
    TcpTransport, Transport, WireMessage, WireOutcome, WireRequest, WireResponse,
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
    pub use overseerd_core::dirs::*;
}

/// Config source-format markers for `ConfigManager<F>`: [`Toml`], [`Yaml`], and the
/// format-erased [`Dynamic`] (which tries every enabled format).
///
/// [`Toml`]: overseerd_core::config::Toml
/// [`Yaml`]: overseerd_core::config::Yaml
/// [`Dynamic`]: overseerd_core::config::Dynamic
pub mod config {
    pub use overseerd_core::config::{Dynamic, Format, FormatId, Toml};

    #[cfg(feature = "yaml")]
    pub use overseerd_core::config::Yaml;
}

/// Framework builtins: the seeded [`ShutdownHandle`] injectable, the opt-in
/// [`ServerConfig`] / [`LoggingConfig`] property structs, and the feature-gated
/// `init_tracing` subscriber helper.
pub mod builtins {
    pub use overseerd_core::builtins::{LoggingConfig, ServerConfig};

    #[cfg(feature = "tracing-subscriber")]
    pub use overseerd_core::builtins::{InitTracingError, init_tracing};
}

/// The common imports for building a daemon: `use overseerd::prelude::*;`.
pub mod prelude {
    pub use crate::{
        Cfg, Component, ConfigManager, ConfigProperties, Daemon, Handler, Inject, Payload, Result,
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
