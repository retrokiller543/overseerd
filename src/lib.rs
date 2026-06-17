//! # Overseer
//!
//! A component- and service-oriented RPC framework. Depend on this single crate;
//! it re-exports everything from the implementation crates (`overseer-core` and
//! `overseer-transport`) plus the procedural macros.
//!
//! ## Concepts
//!
//! - **Component** — a singleton dependency. Either *system-constructed* (declared
//!   with [`component`] / a stateful [`service`], built by the container from its
//!   dependencies) or *manually provided* ([`DaemonBuilder::with_component`]).
//! - **Service** — a [`service`] type whose [`handlers`] impls expose `#[rpc]`
//!   methods. A stateful service is also a component (its `&self` is the singleton).
//! - **Container** — holds the constructed instances ([`ComponentContainer`]).
//! - **Registry** — holds the *declarations* ([`DescriptorRegistry`]).
//!
//! ## Example
//!
//! ```ignore
//! use overseer::prelude::*;
//! use serde::{Deserialize, Serialize};
//! use std::sync::Arc;
//!
//! #[derive(Component)]
//! struct Config { greeting: String }
//!
//! #[service(id = "greeter", version = "0.1")]
//! struct Greeter { config: Arc<Config> }
//!
//! #[derive(Deserialize)] struct GreetReq { name: String }
//! #[derive(Serialize)]   struct GreetResp { message: String }
//!
//! #[handlers]
//! impl Greeter {
//!     #[rpc]
//!     async fn greet(&self, Payload(req): Payload<GreetReq>) -> Result<GreetResp> {
//!         Ok(GreetResp { message: format!("{}, {}!", self.config.greeting, req.name) })
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let daemon = Daemon::builder("greeter")
//!         .auto_discover()
//!         .with_component(Config { greeting: "Hello".into() })
//!         .build()
//!         .await?;
//!
//!     daemon.serve(TcpTransport::bind("127.0.0.1:9000").await?).await
//! }
//! ```

// ---------------------------------------------------------------------------
// Core: descriptors, registry, container, daemon, extractors, macros.
// ---------------------------------------------------------------------------
pub use overseer_core::{
    BoxedComponent, Component, ComponentConstructionContext, ComponentContainer, ComponentDescriptor,
    ComponentFactory, ComponentScope, Conn, ConnectionHandler, ConnectionInfo, Daemon,
    DaemonBuilder, DependencyDescriptor, Descriptor, DescriptorRegistry, Error, Extension,
    FromContext, Handler, OperationKind, ParameterDescriptor, ParameterKind, Payload, Result,
    RpcCallContext, RpcDescriptor, RpcGroup, RpcHandler, RpcResponse, RpcRouter, ServiceComponent,
    ServiceDescriptor, ShutdownHandle, ShutdownSignal, TypeDescriptor, component, dispatch_with,
    handlers, rpc, service, type_id_of,
};

// ---------------------------------------------------------------------------
// Transport: server endpoints, client wire protocol, custom-transport traits.
// `Error`/`Result` are intentionally not lifted to the root (the core ones win);
// reach transport's via `overseer::transport`.
// ---------------------------------------------------------------------------
pub use overseer_transport::{
    CallId, CallResult, Connection, IncomingCall, MemoryClient, MemoryConnection,
    MemoryConnectionHandle, MemoryResponder, MemoryTransport, PeerInfo, Respond, TcpTransport,
    Transport, WireMessage, WireOutcome, WireRequest, WireResponse,
};

#[cfg(unix)]
pub use overseer_transport::UnixTransport;

/// The full transport layer, including the framing codec (`transport::protocol::codec`)
/// and connection/responder types for building clients or custom transports.
pub mod transport {
    pub use overseer_transport::*;
}

/// The common imports for building a daemon: `use overseer::prelude::*;`.
pub mod prelude {
    pub use crate::{
        Component, Conn, Daemon, Extension, Handler, Payload, Result, ServiceComponent, component,
        handlers, rpc, service,
    };

    pub use overseer_transport::TcpTransport;

    #[cfg(unix)]
    pub use overseer_transport::UnixTransport;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_exposes_core_types() {
        let td = TypeDescriptor::of::<u8>("byte");

        assert_eq!(td.name, "byte");
        assert_eq!((td.type_id)(), (type_id_of::<u8>)());
    }
}