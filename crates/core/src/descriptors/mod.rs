pub mod component;
pub mod service;
pub mod types;

pub use component::{
    BoxedComponent, ComponentConstructionContext, ComponentDescriptor, ComponentFactory,
    ComponentScope, DependencyDescriptor,
};
pub use service::{
    OperationKind, ParameterDescriptor, ParameterKind, RpcCallContext, RpcDescriptor, RpcHandler,
    RpcResponse, ServiceDescriptor,
};
pub use types::{TypeDescriptor, type_id_of};

/// Top-level inventory entry submitted by proc macros.
///
/// RPCs belong to ServiceDescriptor.rpcs — they are not submitted as separate
/// top-level entries. Only components and services appear here.
pub enum Descriptor {
    Component(&'static ComponentDescriptor),
    Service(&'static ServiceDescriptor),
}

inventory::collect!(Descriptor);
