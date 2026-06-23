//! Core of the Overseerd framework: descriptors, dependency-injection container,
//! RPC router, extractors, and the daemon runtime.
//!
//! Most users should depend on the `overseerd` facade crate rather than this one
//! directly; the facade re-exports this API alongside the transports and macros.
//!
//! The split that runs through the crate: **declarations** live in the
//! [`DescriptorRegistry`] (component/service/RPC descriptors), while **runtime
//! instances** live in the [`ComponentContainer`]. [`Daemon`] ties them together
//! with a [`router::RpcRouter`] and a transport.

pub mod config;
pub mod container;
pub mod daemon;
pub mod descriptors;
pub mod dirs;
pub mod error;
pub mod extract;
pub mod health;
pub mod lifecycle;
pub mod platform;
pub mod registry;
pub mod router;

pub use extract::{
    Cancel, ErrorResponse, FallibleHandler, FromContext, Handler, Inject, Payload, Peer,
    RequestStream, Responder, ResponseError, ResponseStream, Streaming, dispatch_fallible,
    dispatch_with,
};
pub use overseerd_macros::{
    Component, ConfigProperties, component, daemon, handlers, injectable, rpc, service,
};
/// Wire-contract status types and stream item codecs, re-exported from
/// `overseerd-transport` so handler authors import everything from `overseerd_core`.
pub use overseerd_transport::{
    Flags, PredefinedCode, StatusCode, StreamDecode, StreamDecodeError, StreamEncode,
    StreamEncodeError,
};

pub use config::{
    Cfg, ConfigBinding, ConfigBindingDescriptor, ConfigError, ConfigManager, ConfigProperties,
};
pub use container::{ComponentContainer, ScopeContainer};
pub use daemon::{Daemon, DaemonBuilder};
pub use descriptors::{
    BoxedComponent, COMPONENTS, CONFIG_BINDINGS, Cardinality, Component,
    ComponentConstructionContext, ComponentDescriptor, ComponentFactory, ComponentScope,
    DependencyDescriptor, Dynamic, Injectable, OperationKind, PROVIDERS, ParameterDescriptor,
    ParameterKind, Provide, ProviderDescriptor, RPC_GROUPS, RpcCallContext, RpcDescriptor,
    RpcGroup, RpcHandler, RpcOutcome, RpcResponse, SERVICES, ServiceComponent, ServiceDescriptor,
    TypeDescriptor, Wired, Wiring, type_id_of,
};
pub use dirs::{Dir, DirKind, DirectoriesManager};
pub use error::Error;
pub use health::{HealthCheck, HealthCheckFuture, HealthStatus};
pub use lifecycle::{ShutdownHandle, ShutdownSignal};
/// Re-exported so macro-generated code can reference the `#[distributed_slice]`
/// attribute through a stable path without the user crate depending on `linkme`
/// directly.
#[doc(hidden)]
pub use linkme;
pub use registry::DescriptorRegistry;
pub use router::RpcRouter;

pub type Result<T, E = Error> = core::result::Result<T, E>;
