pub mod rpc;

pub use rpc::{
    OperationKind, ParameterDescriptor, ParameterKind, RpcCallContext, RpcDescriptor, RpcHandler,
    RpcResponse,
};

use crate::descriptors::types::TypeDescriptor;

/// Static metadata describing a service and its RPC methods.
#[derive(Debug)]
pub struct ServiceDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub ty: TypeDescriptor,
    pub version: Option<&'static str>,
    pub rpcs: &'static [RpcDescriptor],
}