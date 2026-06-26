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

pub mod builtins;
pub mod config;
pub mod construct;
pub mod container;
pub mod daemon;
pub mod descriptors;
pub mod dirs;
pub mod error;
pub mod extract;
pub mod hooks;
pub mod lifecycle;
pub mod middleware;
pub mod registry;
pub mod router;

pub use construct::{
    Factory, FactoryOutput, FromContainer, dispatch_factory, factory_dependencies,
};
pub use extract::{
    Cancel, ErrorResponse, FallibleHandler, FromContext, Handler, Inject, Payload, Peer,
    RequestStream, Responder, ResponseError, ResponseStream, Streaming, dispatch_fallible,
    dispatch_with,
};
pub use overseerd_macros::{
    component, config, daemon, handlers, injectable, methods, rpc, service,
};
/// Wire-contract status types and stream item codecs, re-exported from
/// `overseerd-transport` so handler authors import everything from `overseerd_core`.
pub use overseerd_transport::{
    Flags, PredefinedCode, StatusCode, StreamDecode, StreamDecodeError, StreamEncode,
    StreamEncodeError,
};

pub use builtins::{LoggingConfig, ServerConfig};
pub use config::{
    Cfg, CfgNext, ChangedBinding, ComponentHookReport, ConfigBinding, ConfigBindingDescriptor,
    ConfigDefaults, ConfigError, ConfigManager, ConfigProperties, ConfigReload, ConfigReloadError,
    ConfigReloadReport, ConfigReloader, DefaultSpec, EnumTag, HookOutcome, ReloadProposal,
    ReloadableConfig,
};
pub use container::{ComponentContainer, ScopeContainer};
pub use hooks::{
    ComponentHooks, HookCall, HookDescriptor, HookKind, HookManager, HookParam, no_hooks,
};
pub use daemon::{Daemon, DaemonBuilder};
pub use descriptors::{
    BoxedComponent, COMPONENTS, CONFIG_BINDINGS, Cardinality, Component,
    ComponentConstructionContext, ComponentDescriptor, ComponentFactories, ComponentFactory,
    ComponentFactoryDescriptor, ComponentScope, Dep, DependencyDescriptor, Descriptor,
    Dynamic, Injectable, Live, LiveRef, OperationKind, PROVIDERS, ParameterDescriptor, ParameterKind, Provide,
    ProviderDescriptor, RpcCallContext, RpcDescriptor, RpcGroup, RpcHandler, RpcOutcome,
    RpcResponse, SERVICES, ServiceComponent, ServiceDescriptor, ServiceRpcs, TypeDescriptor, Wired,
    Wiring, type_id_of,
};
pub use dirs::{Dir, DirKind, DirectoriesManager, DirectoriesResolver};
pub use error::Error;
pub use lifecycle::{ShutdownHandle, ShutdownSignal};
/// Re-exported so macro-generated code can reference the `#[distributed_slice]`
/// attribute through a stable path without the user crate depending on `linkme`
/// directly.
#[doc(hidden)]
pub use linkme;
pub use middleware::{
    ErrorHandler, Guard, GuardLayer, GuardService, RouterService, RpcRequest, RpcService,
};
pub use registry::DescriptorRegistry;
pub use router::RpcRouter;
/// Re-exported so middleware authors can implement `tower::Layer` / `tower::Service`
/// (and reach tower's own layers) without depending on `tower` directly.
pub use tower;

pub type Result<T, E = Error> = core::result::Result<T, E>;
