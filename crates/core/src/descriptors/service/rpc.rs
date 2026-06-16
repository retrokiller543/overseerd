use std::{fmt, future::Future, pin::Pin};

use crate::descriptors::types::TypeDescriptor;

/// Classifies the interaction pattern of an RPC method.
#[derive(Clone, Copy, Debug)]
pub enum OperationKind {
    Command,
    Query,
    Stream,
}

/// Classifies how a parameter value is sourced during an RPC call.
#[derive(Clone, Copy, Debug)]
pub enum ParameterKind {
    Component,
    Payload,
    Context,
    Cancellation,
    Metadata,
}

/// Static metadata describing a single RPC parameter.
#[derive(Clone, Copy, Debug)]
pub struct ParameterDescriptor {
    pub name: &'static str,
    pub kind: ParameterKind,
    pub ty: TypeDescriptor,
}

/// Placeholder context passed to an RPC handler on invocation.
pub struct RpcCallContext {}

/// Placeholder response returned from an RPC handler.
pub struct RpcResponse {}

/// Async function pointer type for dispatching an RPC call.
pub type RpcHandler =
    fn(RpcCallContext) -> Pin<Box<dyn Future<Output = crate::Result<RpcResponse>> + Send>>;

/// Static metadata describing a single RPC method on a service.
pub struct RpcDescriptor {
    pub name: &'static str,
    pub operation: OperationKind,
    pub parameters: &'static [ParameterDescriptor],
    pub output: TypeDescriptor,
    pub handler: RpcHandler,
}

impl fmt::Debug for RpcDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RpcDescriptor")
            .field("name", &self.name)
            .field("operation", &self.operation)
            .field("parameters", &self.parameters)
            .field("output", &self.output)
            .finish_non_exhaustive()
    }
}