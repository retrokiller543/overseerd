//! The Overseerd native RPC protocol, built on the protocol-agnostic `overseerd-app` core.
//!
//! This crate is a [`ProtocolPlugin`]: it adds the RPC router, the `FromContext`
//! extractors, the tower middleware stack, the wire transports, and the serve loop on top
//! of [`overseerd_app`]. Depend on it directly for a self-contained RPC framework
//! (`overseerd` is always present for the core macros + vocabulary), or reach it through
//! the `overseerd` facade's `daemon` feature.

#[cfg(feature = "client")]
pub mod client;
pub mod descriptors;
pub mod error;
pub mod extract;
pub mod middleware;
pub mod plugin;
pub mod protocol;
pub mod router;
pub mod routes;
pub mod scope;

pub use error::{Error, Result};
pub use plugin::{RpcAppBuilder, RpcPlugin};

/// The RPC daemon macros (`#[service]`, `#[handlers]`, `#[rpc]`), owned by this protocol crate.
/// Their generated code roots plugin types at this crate (`::overseerd_rpc::*`) by default, or
/// at `::overseerd::daemon::*` under the `facade` feature — so they work whether `overseerd-rpc`
/// is used directly or through the `overseerd` facade. The core macros (`app!`, `#[component]`,
/// …) come from `overseerd` (the always-present core).
pub use overseerd_rpc_macros::{handlers, rpc, service};
pub use protocol::{Rpc, RpcLimits};
pub use router::RpcRouter;
pub use routes::ResolvedService;

pub use descriptors::{
    Descriptor, OperationKind, ParameterDescriptor, ParameterKind, RpcCallContext, RpcDescriptor,
    RpcGroup, RpcHandler, RpcOutcome, RpcResponse, SERVICES, ServiceDescriptor, ServiceRpcs,
};
pub use extract::{
    Cancel, ErrorResponse, FallibleHandler, FromContext, Handler, Inject, Payload, Peer,
    RequestStream, Responder, ResponseError, ResponseStream, Streaming, dispatch_fallible,
    dispatch_with,
};
pub use middleware::{
    ErrorHandler, Guard, GuardLayer, GuardService, RouterService, RpcRequest, RpcService,
};

/// The RPC app type: an [`App`](overseerd_app::App) specialized to the native [`RpcPlugin`].
/// `App::builder(name)` resolves through this alias without a turbofish.
pub type App = overseerd_app::App<RpcPlugin>;

/// The RPC app builder: [`AppBuilder`](overseerd_app::AppBuilder) specialized to [`RpcPlugin`].
pub type AppBuilder = overseerd_app::AppBuilder<RpcPlugin>;

// Re-export the agnostic app surface so a standalone `overseerd-rpc` user has one import.
pub use overseerd_app::{
    AppRegistry, AppRuntime, LoggingConfig, Plugin, Protocol, ProtocolPlugin, Serve, ServerConfig,
    ShutdownHandle, ShutdownSignal,
};

/// Re-exported so macro-generated code can reach the `#[distributed_slice]` attribute for
/// the per-service / `SERVICES` slices through a stable path.
#[doc(hidden)]
pub use linkme;

/// Re-exported so middleware authors can implement `tower::Layer` / `tower::Service`.
pub use tower;

/// The RPC byte-stream [`ProtocolTransport`](overseerd_client::ProtocolTransport)
/// implementation and its connect helpers. The agnostic client surface (`Client`,
/// `ProtocolTransport`, …) lives in [`overseerd_client`]; this is the RPC carry that plugs
/// into it. Gated behind the `client` feature.
#[cfg(feature = "client")]
pub use client::{RpcResponses, StreamClientTransport, connect_tcp};

#[cfg(all(feature = "client", unix))]
pub use client::connect_unix;

/// The transport substrate, re-exported for generated client code and custom transports.
pub mod transport {
    pub use overseerd_transport::*;
}

/// Re-exported so a `#[rpc(stream)]` handler returning a concrete (un-introspectable) stream
/// type still yields a well-typed client.
#[doc(hidden)]
pub use futures::Stream as __Stream;

/// Re-exported so generated client traits can be annotated `#[async_trait]`.
#[cfg(feature = "client")]
#[doc(hidden)]
pub use async_trait;
