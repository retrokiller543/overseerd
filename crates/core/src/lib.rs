pub mod descriptors;
pub mod error;
pub mod registry;

pub use descriptors::{
    BoxedComponent, ComponentConstructionContext, ComponentDescriptor, ComponentFactory,
    ComponentScope, DependencyDescriptor, Descriptor, OperationKind, ParameterDescriptor,
    ParameterKind, RpcCallContext, RpcDescriptor, RpcHandler, RpcResponse, ServiceDescriptor,
    TypeDescriptor, type_id_of,
};
pub use error::Error;
pub use registry::Registry;

pub type Result<T> = std::result::Result<T, Error>;