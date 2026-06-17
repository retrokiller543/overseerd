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
pub use extract::{Conn, Extension, FromContext, Handler, Payload, dispatch_with};
pub use overseer_macros::{handlers, rpc, service};

/// Re-exported so macro-generated code can call `inventory::submit!` through a
/// stable path without the user crate depending on `inventory` directly.
#[doc(hidden)]
pub use inventory;
pub use container::Container;
pub use daemon::{Daemon, DaemonBuilder};
pub use descriptors::{
    BoxedComponent, ComponentConstructionContext, ComponentDescriptor, ComponentFactory,
    ComponentScope, DependencyDescriptor, Descriptor, OperationKind, ParameterDescriptor,
    ParameterKind, RpcCallContext, RpcDescriptor, RpcGroup, RpcHandler, RpcResponse,
    ServiceDescriptor, TypeDescriptor, type_id_of,
};
pub use error::Error;
pub use lifecycle::{ShutdownHandle, ShutdownSignal};
pub use registry::Registry;
pub use router::RpcRouter;

pub type Result<T> = std::result::Result<T, Error>;
