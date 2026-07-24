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
pub use builtins::{LogFormat, LoggingConfig, ServerConfig, SpanEvents};
pub use error::{Error, Result};
pub use host::{
    AppHost, AppStage, BootstrapContext, Built, ExecutionMode, HostError, Initial, LifecyclePhase,
    PhaseError, PreBuild, Setup, build_host, build_host_context, build_prepared_host, prepare_host,
    prepare_host_context, prepare_setup_host_context, resolve_host_dependency, serve_host,
    setup_host, setup_host_context,
};
#[cfg(feature = "cli")]
pub use host::{
    BootstrapError, BootstrapOptions, BootstrapPolicy, BootstrapState, CliCommand,
    CliDefinitionError, CliError, ColorChoice, CommandContext, CommandContextError, CommandError,
    CommandPhase, bootstrap_application, bootstrap_application_with_policy,
    configure_bootstrap_config, configure_bootstrap_directories, finalize_bootstrap, validate_cli,
};
pub use lifecycle::{ShutdownHandle, ShutdownSignal};
pub use protocol::{Plugin, PreBuildContext, Protocol, ProtocolPlugin, Serve, ValidationContext};
pub use registry::AppRegistry;
pub use runtime::AppRuntime;

#[cfg(feature = "cli")]
pub use clap;
