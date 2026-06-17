use std::{fmt, future::Future, pin::Pin};

use crate::descriptors::types::TypeDescriptor;

/// The interaction pattern of an RPC method, following gRPC's four kinds.
///
/// Only `Unary` is served today. The streaming variants are the target of the
/// streaming plan (`specs/002-streaming-rpcs/plan.md`); they are not yet
/// produced by the `#[rpc]` macro nor handled by the runtime.
#[derive(Clone, Copy, Debug)]
pub enum OperationKind {
    /// One request, one response.
    Unary,
    /// One request, a stream of responses.
    ServerStream,
    /// A stream of requests, one response.
    ClientStream,
    /// A bidirectional stream of requests and responses.
    BidiStream,
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

/// Context passed to an RPC handler on invocation.
///
/// Carries the raw postcard-encoded payload bytes and the connection-scoped
/// context. Handler extractors (e.g. `Payload<T>`) deserialize from `payload`
/// into the user's own types; `connection` provides per-connection data such
/// as auth context or rate-limit state.
pub struct RpcCallContext {
    pub payload: Vec<u8>,
    pub connection: std::sync::Arc<crate::connection::ConnectionInfo>,
}

/// The response returned by an RPC handler.
///
/// `payload` holds postcard-encoded response bytes. An empty payload is valid
/// for commands that return no meaningful data.
pub struct RpcResponse {
    pub payload: Vec<u8>,
}

impl Default for RpcResponse {
    fn default() -> Self {
        Self {
            payload: Vec::new(),
        }
    }
}

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
