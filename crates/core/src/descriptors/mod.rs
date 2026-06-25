pub mod component;
pub mod service;
pub mod types;

pub use component::{
    BoxedComponent, Cardinality, Component, ComponentConstructionContext, ComponentDescriptor,
    ComponentFactory, ComponentFactoryDescriptor, ComponentScope, Dep, DependencyDescriptor,
    Dynamic, Injectable, Live, LiveRef, Provide, ProviderDescriptor, ServiceComponent, Wired, Wiring,
};
pub use service::{
    OperationKind, ParameterDescriptor, ParameterKind, RpcCallContext, RpcDescriptor, RpcGroup,
    RpcHandler, RpcOutcome, RpcResponse, ServiceDescriptor,
};
pub use types::{TypeDescriptor, type_id_of};

/// Connects a type to a static descriptor of kind `D`, enabling type-to-descriptor
/// lookups where the type is known but its link-time descriptor would otherwise
/// only be reachable through a slice.
///
/// A type may implement this once per descriptor kind: a `#[service]` carries both
/// `Descriptor<ComponentDescriptor>` (its construction factory) and
/// `Descriptor<ServiceDescriptor>` (its identity header). This is what lets
/// [`DaemonBuilder`](crate::DaemonBuilder) register a component/service/config
/// *by type* (`builder.service::<T>()`) instead of by `&'static` descriptor or by
/// global auto-discovery.
///
/// RPC groups are deliberately *not* expressed this way: a single service may span
/// several `#[handlers]` blocks, so the relationship is one-to-many. Reach them via
/// [`ServiceRpcs::rpc_groups`] instead.
pub trait Descriptor<D> {
    const DESCRIPTOR: D;
}

/// A component type's own construction factories.
///
/// Implemented for each `#[component]`/`#[service]` by the macro to return that
/// type's `{Type}Factories` distributed slice — the field-injection default plus any
/// `#[init]` / `factory = ..` contributions, which each append to it. The owning
/// [`ComponentDescriptor`] stores this as a fn pointer
/// (`factories: <T as ComponentFactories>::factories`), so the type-erased registry
/// reaches a type's factories without holding its type. Mirrors [`ServiceRpcs`].
pub trait ComponentFactories {
    /// Every factory contributed to this component type.
    fn factories() -> &'static [ComponentFactoryDescriptor];
}

/// A service type's own RPC groups.
///
/// Implemented for each `#[service]` by the macro to return that service's
/// `{Service}Rpcs` distributed slice — the slice every one of its `#[handlers]`
/// blocks appends to, so multiple blocks compose into the single returned slice.
/// The owning [`ServiceDescriptor`] stores this method as a fn pointer
/// (`rpcs: <T as ServiceRpcs>::rpc_groups`), so the type-erased registry can reach a
/// service's groups without holding its type.
pub trait ServiceRpcs {
    /// Every RPC group contributed to this service across its `#[handlers]` blocks.
    fn rpc_groups() -> &'static [RpcGroup];
}

/// Link-time descriptor registries, one homogeneous slice per descriptor kind.
///
/// Proc macros register one element each via `#[linkme::distributed_slice(..)]`:
/// a `#[component]`/`#[service]` factory (and an `#[init]` constructor) into
/// [`COMPONENTS`] and a `#[service]` header into [`SERVICES`].
/// [`DescriptorRegistry::collect`] reads the assembled slices. Unlike a single
/// tagged-enum stream, each slice is homogeneous and assembled at link time with
/// no per-startup registration walk. RPC groups are *not* collected globally: each
/// service owns a `{Service}Rpcs` slice its `#[handlers]` blocks append to, reached
/// through [`ServiceDescriptor::rpcs`].
#[linkme::distributed_slice]
pub static COMPONENTS: [ComponentDescriptor];

/// Link-time registry of every discovered [`ServiceDescriptor`]. See [`COMPONENTS`].
#[linkme::distributed_slice]
pub static SERVICES: [ServiceDescriptor];

/// Link-time registry of every discovered [`ProviderDescriptor`] (a component
/// declaring `provide = dyn Trait`). See [`COMPONENTS`].
#[linkme::distributed_slice]
pub static PROVIDERS: [ProviderDescriptor];

/// Link-time registry of every auto-registered config binding (a
/// `#[config(path = "..")]` type). See
/// [`COMPONENTS`].
#[linkme::distributed_slice]
pub static CONFIG_BINDINGS: [crate::config::ConfigBindingDescriptor];
