//! The Overseerd daemon runtime and RPC adapter.
//!
//! This crate is the protocol layer: it ties the DI engine, the config system, the hook
//! system, and a transport together into a running daemon. It owns the [`AppBuilder`],
//! the RPC [`router`], the request [`extract`]ors, the [`middleware`] stack, the serve
//! loop, and the full [`DescriptorRegistry`] (component graph via the DI engine, plus
//! service/RPC and config-binding validation).
//!
//! The protocol-agnostic pieces below it — DI, config, hooks, dirs — are usable on their
//! own; this crate is what an RPC daemon adds on top. A different protocol (HTTP, gRPC)
//! would be a sibling of this crate over the same foundation.

pub mod builtins;
pub mod daemon;
pub mod descriptors;
pub mod error;
pub mod extract;
pub mod lifecycle;
pub mod middleware;
pub mod protocol;
pub mod registry;
pub mod router;
pub mod runtime;
pub mod scope;

pub use error::Error;

/// The daemon-layer result type.
pub type Result<T, E = Error> = core::result::Result<T, E>;

pub use builtins::{LoggingConfig, ServerConfig};
pub use daemon::{App, AppBuilder};
pub use descriptors::{
    Descriptor, OperationKind, ParameterDescriptor, ParameterKind, RpcCallContext, RpcDescriptor,
    RpcGroup, RpcHandler, RpcOutcome, RpcResponse, SERVICES, ServiceDescriptor, ServiceRpcs,
};
pub use extract::{
    Cancel, ErrorResponse, FallibleHandler, FromContext, Handler, Inject, Payload, Peer,
    RequestStream, Responder, ResponseError, ResponseStream, Streaming, dispatch_fallible,
    dispatch_with,
};
pub use lifecycle::{ShutdownHandle, ShutdownSignal};
pub use middleware::{
    ErrorHandler, Guard, GuardLayer, GuardService, RouterService, RpcRequest, RpcService,
};
pub use daemon::RpcPlugin;
pub use protocol::{Plugin, Protocol, ProtocolPlugin, Rpc, Serve};
pub use registry::{DescriptorRegistry, ResolvedService};
pub use router::RpcRouter;
pub use runtime::AppRuntime;

/// Re-exported so macro-generated code can reach the `#[distributed_slice]` attribute for
/// the `SERVICES` slice through a stable path.
#[doc(hidden)]
pub use linkme;

/// Re-exported so middleware authors can implement `tower::Layer` / `tower::Service`
/// without depending on `tower` directly.
pub use tower;
