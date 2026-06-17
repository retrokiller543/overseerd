pub mod component;
pub mod service;
pub mod types;

pub use component::{
    BoxedComponent, Component, ComponentConstructionContext, ComponentDescriptor, ComponentFactory,
    ComponentScope, DependencyDescriptor, ServiceComponent,
};
pub use service::{
    OperationKind, ParameterDescriptor, ParameterKind, RpcCallContext, RpcDescriptor, RpcGroup,
    RpcHandler, RpcOutcome, RpcResponse, ServiceDescriptor,
};
pub use types::{TypeDescriptor, type_id_of};

/// Top-level inventory entry submitted by proc macros.
///
/// A `Service` declares a service's identity (tied to its type); each `Rpcs`
/// group contributes RPC methods to the service of a matching type, so one
/// service may span several impl blocks. `Component` registers a constructable
/// singleton (e.g. a stateful service holding common deps).
pub enum Descriptor {
    Component(&'static ComponentDescriptor),
    Service(&'static ServiceDescriptor),
    Rpcs(&'static RpcGroup),
}

inventory::collect!(Descriptor);
