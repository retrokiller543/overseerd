pub mod component;
pub mod service;
pub mod types;

pub use component::{
    BoxedComponent, Cardinality, Component, ComponentConstructionContext, ComponentDescriptor,
    ComponentFactory, ComponentScope, DependencyDescriptor, Dynamic, Injectable, Provide,
    ProviderDescriptor, ServiceComponent, Wired, Wiring,
};
pub use service::{
    OperationKind, ParameterDescriptor, ParameterKind, RpcCallContext, RpcDescriptor, RpcGroup,
    RpcHandler, RpcOutcome, RpcResponse, ServiceDescriptor,
};
pub use types::{TypeDescriptor, type_id_of};

/// Link-time descriptor registries, one homogeneous slice per descriptor kind.
///
/// Proc macros register one element each via `#[linkme::distributed_slice(..)]`:
/// a `#[component]`/`#[service]` factory (and an `#[init]` constructor) into
/// [`COMPONENTS`], a `#[service]` header into [`SERVICES`], and each `#[handlers]`
/// block's methods into [`RPC_GROUPS`] (so one service may span several impls).
/// [`DescriptorRegistry::collect`] reads the assembled slices. Unlike a single
/// tagged-enum stream, each slice is homogeneous and assembled at link time with
/// no per-startup registration walk.
#[linkme::distributed_slice]
pub static COMPONENTS: [ComponentDescriptor];

/// Link-time registry of every discovered [`ServiceDescriptor`]. See [`COMPONENTS`].
#[linkme::distributed_slice]
pub static SERVICES: [ServiceDescriptor];

/// Link-time registry of every discovered [`RpcGroup`]. See [`COMPONENTS`].
#[linkme::distributed_slice]
pub static RPC_GROUPS: [RpcGroup];

/// Link-time registry of every discovered [`ProviderDescriptor`] (a component
/// declaring `provide = dyn Trait`). See [`COMPONENTS`].
#[linkme::distributed_slice]
pub static PROVIDERS: [ProviderDescriptor];
