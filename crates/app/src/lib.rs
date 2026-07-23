//! The Overseerd protocol-agnostic application core.
//!
//! This crate ties the DI engine, config, hooks, and dirs into a runnable [`App`] that is
//! generic over the [`ProtocolPlugin`] it installs. It owns the [`AppBuilder`], the agnostic
//! [`AppRegistry`], scope planning, the lifecycle/serve envelope, the [`AppRuntime`] handle
//! a protocol drives requests through, and the [`Plugin`]/[`Protocol`]/[`Serve`] seam.
//!
//! It is *protocol-agnostic*: it knows nothing of RPC, HTTP, or any wire format. A protocol
//! (the native RPC daemon, a future axum binding) is a sibling crate that implements these
//! traits over this foundation.

pub mod app;
pub mod builtins;
pub mod error;
pub mod host;
pub mod lifecycle;
pub mod protocol;
pub mod registry;
pub mod runtime;

pub use app::{App, AppBuilder, PreparedApp};
pub use builtins::{LoggingConfig, ServerConfig};
pub use error::{Error, Result};
pub use host::{AppHost, BootstrapContext, ExecutionMode, LifecyclePhase, PhaseError};
pub use lifecycle::{ShutdownHandle, ShutdownSignal};
pub use protocol::{Plugin, PreBuildContext, Protocol, ProtocolPlugin, Serve};
pub use registry::AppRegistry;
pub use runtime::AppRuntime;
