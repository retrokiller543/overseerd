//! Service and RPC descriptors — the daemon's protocol metadata over the DI engine.

pub mod rpc;

pub use rpc::{
    OperationKind, ParameterDescriptor, ParameterKind, RpcCallContext, RpcDescriptor, RpcHandler,
    RpcOutcome, RpcResponse,
};

pub use overseerd_core::Descriptor;
use overseerd_core::TypeDescriptor;

/// Identity of a service, tied to its implementing type, carrying a handle to its
/// own RPC surface.
///
/// `rpcs` points at [`ServiceRpcs::rpc_groups`] for the service's type — which returns the
/// `{Service}Rpcs` slice every one of its `#[handlers]` blocks appends to. The service thus
/// *owns* its methods: registering it registers its RPCs, with no separate global RPC
/// registry to double-count. It is a fn pointer (not a `&'static [RpcGroup]`) because the
/// macro-emitted descriptor is a `const`, which cannot reference the `static` slice directly.
/// `Copy` so the registry can own a flat `Vec`, mixing link-time-collected and
/// runtime-registered headers.
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

/// A service type's own RPC groups.
///
/// Implemented for each `#[service]` by the macro to return that service's `{Service}Rpcs`
/// distributed slice — the slice every one of its `#[handlers]` blocks appends to, so
/// multiple blocks compose into the single returned slice. The owning [`ServiceDescriptor`]
/// stores this method as a fn pointer (`rpcs: <T as ServiceRpcs>::rpc_groups`), so the
/// type-erased registry can reach a service's groups without holding its type.
pub trait ServiceRpcs {
    /// Every RPC group contributed to this service across its `#[handlers]` blocks.
    fn rpc_groups() -> &'static [RpcGroup];
}

/// Link-time registry of every discovered [`ServiceDescriptor`].
///
/// A `#[service]` registers its header here via `#[linkme::distributed_slice(SERVICES)]`;
/// [`DescriptorRegistry::collect`](crate::registry::DescriptorRegistry::collect) reads the
/// assembled slice. RPC groups are *not* collected globally: each service owns a
/// `{Service}Rpcs` slice its `#[handlers]` blocks append to, reached through
/// [`ServiceDescriptor::rpcs`].
#[linkme::distributed_slice]
pub static SERVICES: [ServiceDescriptor];
