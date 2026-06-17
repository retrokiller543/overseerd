pub mod rpc;

pub use rpc::{
    OperationKind, ParameterDescriptor, ParameterKind, RpcCallContext, RpcDescriptor, RpcHandler,
    RpcResponse,
};

use crate::descriptors::types::TypeDescriptor;

/// Static identity of a service, tied to its implementing type.
///
/// RPCs are contributed separately via [`RpcGroup`]s (one per `#[handlers]`
/// impl block) and assembled against this descriptor by matching `ty`, so a
/// single service may span several impl blocks.
#[derive(Debug)]
pub struct ServiceDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub ty: TypeDescriptor,
    pub version: Option<&'static str>,
}

/// A set of RPCs contributed to the service of type `service` by one impl block.
#[derive(Debug)]
pub struct RpcGroup {
    pub service: TypeDescriptor,
    pub rpcs: &'static [RpcDescriptor],
}
