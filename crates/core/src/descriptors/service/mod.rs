pub mod rpc;

pub use rpc::{
    OperationKind, ParameterDescriptor, ParameterKind, RpcCallContext, RpcDescriptor, RpcHandler,
    RpcOutcome, RpcResponse,
};

use crate::descriptors::types::TypeDescriptor;

/// Identity of a service, tied to its implementing type, carrying a handle to its
/// own RPC surface.
///
/// `rpcs` points at [`ServiceRpcs::rpc_groups`](crate::descriptors::ServiceRpcs::rpc_groups)
/// for the service's type — which returns the `{Service}Rpcs` slice every one of its
/// `#[handlers]` blocks appends to. The service thus *owns* its methods: registering
/// it registers its RPCs, with no separate global RPC registry to double-count. It is
/// a fn pointer (not a `&'static [RpcGroup]`) because the macro-emitted descriptor is
/// a `const`, which cannot reference the `static` slice directly. `Copy` so the
/// registry can own a flat `Vec`, mixing link-time-collected and runtime-registered
/// headers.
#[derive(Clone, Copy, Debug)]
pub struct ServiceDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub ty: TypeDescriptor,
    pub version: Option<&'static str>,
    pub rpcs: fn() -> &'static [RpcGroup],
}

/// A set of RPCs contributed to the service of type `service` by one impl block.
#[derive(Clone, Copy, Debug)]
pub struct RpcGroup {
    pub service: TypeDescriptor,
    pub rpcs: &'static [RpcDescriptor],
}
