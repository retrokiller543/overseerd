pub mod rpc;

pub use rpc::{
    OperationKind, ParameterDescriptor, ParameterKind, RpcCallContext, RpcDescriptor, RpcHandler,
    RpcOutcome, RpcResponse,
};

use crate::descriptors::types::TypeDescriptor;
use crate::ServiceComponent;

/// Identity of a service, tied to its implementing type.
///
/// RPCs are contributed separately via [`RpcGroup`]s (one per `#[handlers]`
/// impl block) and assembled against this descriptor by matching `ty`, so a
/// single service may span several impl blocks. `Copy` so the registry can own
/// a flat `Vec`, mixing link-time-collected and runtime-registered headers.
#[derive(Clone, Copy, Debug)]
pub struct ServiceDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub ty: TypeDescriptor,
    pub version: Option<&'static str>,
}

impl ServiceDescriptor {
    pub const fn of<T: ServiceComponent>() -> Self {
        Self {
            id: T::ID,
            name: T::NAME,
            ty: TypeDescriptor::of::<T>(T::NAME),
            version: T::VERSION,
        }
    }
}

/// A set of RPCs contributed to the service of type `service` by one impl block.
#[derive(Clone, Copy, Debug)]
pub struct RpcGroup {
    pub service: TypeDescriptor,
    pub rpcs: &'static [RpcDescriptor],
}
