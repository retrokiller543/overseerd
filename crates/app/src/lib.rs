//! The Overseerd protocol-agnostic application core.
//!
//! This crate ties the DI engine, config, hooks, and dirs into a runnable [`App`] that is
//! generic over the [`ProtocolDefinition`] it prepares. It owns the [`AppBuilder`], the agnostic
//! [`AppRegistry`], scope planning, the lifecycle/serve envelope, the [`AppRuntime`] handle
//! a protocol drives requests through, and the protocol state/serve seams.
//!
//! It is *protocol-agnostic*: it knows nothing of RPC, HTTP, or any wire format. A protocol
//! (the native RPC daemon, a future axum binding) is a sibling crate that implements these
//! traits over this foundation.

pub mod app;
pub mod builtins;
pub mod composition;
pub mod error;
pub mod host;
pub mod lifecycle;
pub mod plugin;
pub mod protocol;
pub mod registry;
pub mod runtime;
pub mod scope;

pub use app::{App, AppBuilder, PreparedApp};
pub use builtins::{LogFormat, LoggingConfig, ServerConfig, SpanEvents};
pub use composition::{
    CompositionDiagnostic, CompositionDiagnostics, CompositionDirective, CompositionEdge,
    CompositionPhase, CompositionTarget, ContributionId, ContributionProvenance, Contributor,
    EarlyPluginPlan, IdErrorKind, InstallationOrigin, InstallationProvenance, InvalidCompositionId,
    PluginDeclaration, PluginId, PluginRelation, PluginResolutionPlan, PluginSlotId, ProtocolId,
    RelationKind, RelationTarget, ReplacementDecision, ResolvedPlugin, SlotPolicy,
    SuppressionDecision, extend_late_plugins, resolve_early_plugins,
};
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
pub use overseerd_core::{Scope, ScopeId, StaticScope, namespaced_id};
pub use plugin::{
    ApplicationPluginRegistrar, EffectivePluginPlan, Plugin, PluginContribution,
    PluginContributionKind, PluginContributions, PluginPlanError, PluginWithOptions,
    ProtocolPluginRegistrar,
};
pub use protocol::{
    PreBuildContext, PreparedProtocol, ProtocolDefinition, ProtocolRuntime, Serve,
    ValidationContext,
};
pub use registry::AppRegistry;
pub use runtime::AppRuntime;
pub use scope::{
    PreparedScopeTopology, ScopeBoundary, ScopeParent, ScopeTopology, ScopeTopologyError,
};

#[cfg(feature = "cli")]
pub use clap;
