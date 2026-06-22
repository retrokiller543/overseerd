//! Core of the Overseer framework: descriptors, dependency-injection container,
//! RPC router, extractors, and the daemon runtime.
//!
//! Most users should depend on the `overseer` facade crate rather than this one
//! directly; the facade re-exports this API alongside the transports and macros.
//!
//! The split that runs through the crate: **declarations** live in the
//! [`DescriptorRegistry`] (component/service/RPC descriptors), while **runtime
//! instances** live in the [`ComponentContainer`]. [`Daemon`] ties them together
//! with a [`router::RpcRouter`] and a transport.

pub mod connection;
pub mod container;
pub mod daemon;
pub mod descriptors;
pub mod error;
pub mod extract;
pub mod lifecycle;
pub mod registry;
pub mod router;

pub use connection::{ConnectionHandler, ConnectionInfo};
pub use extract::{
    Cancel, Conn, ErrorResponse, Extension, FallibleHandler, FromContext, Handler, Payload,
    Responder, ResponseError, ResponseStream, Streaming, dispatch_fallible, dispatch_with,
};
pub use overseer_macros::{Component, component, handlers, rpc, service};
/// Wire-contract status types, re-exported from `overseer-transport` so handler
/// authors import everything from `overseer_core`.
pub use overseer_transport::{Flags, PredefinedCode, StatusCode};

pub use container::ComponentContainer;
pub use daemon::{Daemon, DaemonBuilder};
pub use descriptors::{
    COMPONENTS, PROVIDERS, RPC_GROUPS, SERVICES, BoxedComponent, Cardinality, Component,
    ComponentConstructionContext, ComponentDescriptor, ComponentFactory, ComponentScope,
    DependencyDescriptor, Dynamic, Injectable, OperationKind, ParameterDescriptor, ParameterKind,
    ProviderDescriptor, RpcCallContext, RpcDescriptor, RpcGroup, RpcHandler, RpcOutcome,
    RpcResponse, ServiceComponent, ServiceDescriptor, TypeDescriptor, type_id_of,
};
pub use error::Error;
/// Re-exported so macro-generated code can reference the `#[distributed_slice]`
/// attribute through a stable path without the user crate depending on `linkme`
/// directly.
#[doc(hidden)]
pub use linkme;
pub use lifecycle::{ShutdownHandle, ShutdownSignal};
pub use registry::DescriptorRegistry;
pub use router::RpcRouter;

pub type Result<T, E = Error> = core::result::Result<T, E>;
