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
/// [`ServiceRpcs::rpc_groups`] instead, which selects them from [`RPC_GROUPS`] by
/// owning type.
pub trait Descriptor<D> {
    const DESCRIPTOR: D;
}

/// A service type's runtime access to the [`RpcGroup`]s contributed to it.
///
/// Blanket-implemented for every type carrying a [`Descriptor<ServiceDescriptor>`],
/// so any `#[service]` gets it for free. Because RPC groups are one-to-many per
/// service (one per `#[handlers]` block, possibly across modules) they cannot be a
/// single `Descriptor` const; instead [`rpc_groups`](Self::rpc_groups) selects every
/// group whose owning service is `Self` from the link-time [`RPC_GROUPS`] set —
/// `group.service` already records the owning type, so multiple blocks compose with
/// no macro coordination.
pub trait ServiceRpcs: Descriptor<ServiceDescriptor> + 'static {
    /// The RPC groups owned by this service type, across all of its `#[handlers]`
    /// blocks.
    fn rpc_groups() -> impl Iterator<Item = &'static RpcGroup> {
        let ty = type_id_of::<Self>();

        RPC_GROUPS
            .iter()
            .filter(move |group| (group.service.type_id)() == ty)
    }
}

impl<T: Descriptor<ServiceDescriptor> + 'static> ServiceRpcs for T {}

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

/// Link-time registry of every auto-registered config binding (a
/// `#[derive(ConfigProperties)]` type with a `#[config(path = "..")]`). See
/// [`COMPONENTS`].
#[linkme::distributed_slice]
pub static CONFIG_BINDINGS: [crate::config::ConfigBindingDescriptor];
