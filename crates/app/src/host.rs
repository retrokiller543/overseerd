//! Reusable application-host lifecycle contracts.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;

use crate::{AppBuilder, ProtocolPlugin};

#[cfg(feature = "cli")]
use overseerd_config::{ConfigManager, Dynamic};
#[cfg(feature = "cli")]
use overseerd_dirs::DirectoriesManager;

#[cfg(feature = "cli")]
use crate::{LogFormat, LoggingConfig};

/// Selects whether a host is executing normally or preparing metadata for developer tooling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExecutionMode {
    /// Normal application construction and execution.
    Run,
    /// Side-effect-free application preparation for developer tooling.
    Tooling,
}

impl ExecutionMode {
    /// Whether this execution may proceed into normal runtime behavior.
    pub fn is_run(self) -> bool {
        matches!(self, Self::Run)
    }

    /// Whether this execution is restricted to tooling-safe preparation.
    pub fn is_tooling(self) -> bool {
        matches!(self, Self::Tooling)
    }
}

/// Identifies the lifecycle phase in which a host operation failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LifecyclePhase {
    /// Bootstrap context creation.
    Setup,
    /// Application builder creation and configuration.
    Configure,
    /// Final builder customization before validation.
    BeforeBuild,
    /// Registration and validation without component construction.
    Prepare,
    /// Component and protocol construction.
    Build,
    /// Post-construction customization.
    AfterBuild,
    /// Long-running protocol execution.
    Serve,
}

impl fmt::Display for LifecyclePhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Setup => "setup",
            Self::Configure => "configure",
            Self::BeforeBuild => "before_build",
            Self::Prepare => "prepare",
            Self::Build => "build",
            Self::AfterBuild => "after_build",
            Self::Serve => "serve",
        };

        formatter.write_str(name)
    }
}

/// Typed values and execution metadata shared across generated host lifecycle phases.
pub struct BootstrapContext {
    mode: ExecutionMode,
    extensions: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl BootstrapContext {
    /// Creates an empty context for `mode`.
    pub fn new(mode: ExecutionMode) -> Self {
        Self {
            mode,
            extensions: HashMap::new(),
        }
    }

    /// The execution mode selected by the host runner.
    pub fn mode(&self) -> ExecutionMode {
        self.mode
    }

    /// Inserts a typed lifecycle value, returning the previous value of that type if present.
    pub fn insert<T: Send + Sync + 'static>(&mut self, value: T) -> Option<T> {
        self.extensions
            .insert(TypeId::of::<T>(), Box::new(value))
            .and_then(|previous| previous.downcast::<T>().ok())
            .map(|previous| *previous)
    }

    /// Borrows a typed lifecycle value.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.extensions.get(&TypeId::of::<T>())?.downcast_ref()
    }

    /// Mutably borrows a typed lifecycle value.
    pub fn get_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        self.extensions.get_mut(&TypeId::of::<T>())?.downcast_mut()
    }

    /// Removes and returns a typed lifecycle value.
    pub fn remove<T: Send + Sync + 'static>(&mut self) -> Option<T> {
        self.extensions
            .remove(&TypeId::of::<T>())?
            .downcast::<T>()
            .ok()
            .map(|value| *value)
    }

    /// Resolved framework bootstrap state, when generated CLI bootstrap has run.
    #[cfg(feature = "cli")]
    pub fn bootstrap(&self) -> Option<&BootstrapState> {
        self.get()
    }

    /// Mutable resolved framework bootstrap state.
    #[cfg(feature = "cli")]
    pub fn bootstrap_mut(&mut self) -> Option<&mut BootstrapState> {
        self.get_mut()
    }

    /// Replaces this context's generated CLI bootstrap state.
    #[cfg(feature = "cli")]
    pub fn set_bootstrap(&mut self, state: BootstrapState) {
        self.insert(state);
    }
}

/// A lifecycle failure tagged with the phase that produced it.
#[derive(Debug, thiserror::Error)]
#[error("{phase} phase failed: {source}")]
pub struct PhaseError {
    phase: LifecyclePhase,
    #[source]
    source: Box<dyn std::error::Error + Send + Sync>,
}

/// Host-runner failures that occur outside user lifecycle callbacks.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HostError {
    /// Tooling execution attempted to construct ordinary components or a served protocol.
    #[error("tooling mode cannot construct application components or protocols")]
    ToolingConstruction,
}

/// Controls when generated CLI output uses ANSI color.
#[cfg(feature = "cli")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
#[non_exhaustive]
pub enum ColorChoice {
    /// Detect color support from the current output stream.
    #[default]
    Auto,
    /// Always emit ANSI color.
    Always,
    /// Never emit ANSI color.
    Never,
}

/// Resolved framework-owned state produced by generated CLI bootstrap.
#[cfg(feature = "cli")]
pub struct BootstrapState {
    options: BootstrapOptions,
    config_path: std::path::PathBuf,
    profiles: Vec<String>,
    directories: DirectoriesManager,
    config: Option<ConfigManager<Dynamic>>,
    logging: LoggingConfig,
    color: ColorChoice,
    tracing_installed: bool,
}

#[cfg(feature = "cli")]
impl BootstrapState {
    /// Parsed protocol-neutral command-line options.
    pub fn options(&self) -> &BootstrapOptions {
        &self.options
    }

    /// Effective config file or directory.
    pub fn config_path(&self) -> &std::path::Path {
        &self.config_path
    }

    /// Effective ordered profile list.
    pub fn profiles(&self) -> &[String] {
        &self.profiles
    }

    /// Effective tracing configuration after CLI/environment overrides.
    pub fn logging(&self) -> &LoggingConfig {
        &self.logging
    }

    /// Effective generated CLI color behavior.
    pub fn color(&self) -> ColorChoice {
        self.color
    }

    /// Whether generated bootstrap installed the global tracing subscriber.
    pub fn tracing_installed(&self) -> bool {
        self.tracing_installed
    }

    /// Resolved application directories.
    pub fn directories(&self) -> &DirectoriesManager {
        &self.directories
    }

    /// Moves the merged config manager into an application builder once.
    pub fn take_config(&mut self) -> Option<ConfigManager<Dynamic>> {
        self.config.take()
    }
}

/// Failures from generated framework bootstrap.
#[cfg(feature = "cli")]
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BootstrapError {
    /// Safe platform directories could not be resolved.
    #[error("failed to resolve application directories: {0}")]
    Directories(#[source] std::io::Error),

    /// Configuration loading or extraction failed.
    #[error(transparent)]
    Config(#[from] overseerd_config::ConfigError),

    /// An environment-provided log format was not recognized.
    #[error("unknown log format '{value}', expected one of: full, compact, pretty, json")]
    LogFormat { value: String },

    /// The generated bootstrap could not install tracing.
    #[cfg(feature = "tracing-subscriber")]
    #[error(transparent)]
    Tracing(#[from] crate::builtins::InitTracingError),
}

#[cfg(feature = "cli")]
#[derive(Default)]
struct BootstrapEnvironment {
    config: Option<std::ffi::OsString>,
    profiles: Option<String>,
    rust_log: Option<String>,
    log_format: Option<String>,
    no_color: bool,
    color_force: Option<String>,
}

#[cfg(feature = "cli")]
impl BootstrapEnvironment {
    fn capture() -> Self {
        Self {
            config: std::env::var_os("OVERSEERD_CONFIG"),
            profiles: std::env::var("OVERSEERD_PROFILES").ok(),
            rust_log: std::env::var("RUST_LOG").ok(),
            log_format: std::env::var("OVERSEERD_LOG_FORMAT").ok(),
            no_color: std::env::var_os("NO_COLOR").is_some(),
            color_force: std::env::var("CLICOLOR_FORCE").ok(),
        }
    }
}

/// Resolves generated application bootstrap without parsing process arguments.
#[cfg(feature = "cli")]
pub fn bootstrap_application(
    application: &str,
    mode: ExecutionMode,
    options: BootstrapOptions,
) -> Result<BootstrapContext, BootstrapError> {
    bootstrap_application_with_env(application, mode, options, BootstrapEnvironment::capture())
}

/// Applies generated bootstrap managers to a protocol-specific application builder.
#[cfg(feature = "cli")]
pub fn configure_bootstrap<P: ProtocolPlugin>(
    context: &mut BootstrapContext,
    builder: AppBuilder<P>,
) -> AppBuilder<P> {
    let Some(state) = context.bootstrap_mut() else {
        return builder;
    };
    let builder = builder.directories(state.directories().clone());

    match state.take_config() {
        Some(config) => builder.config_source(config),
        None => builder,
    }
}

#[cfg(feature = "cli")]
fn bootstrap_application_with_env(
    application: &str,
    mode: ExecutionMode,
    options: BootstrapOptions,
    environment: BootstrapEnvironment,
) -> Result<BootstrapContext, BootstrapError> {
    let directories =
        DirectoriesManager::try_for_app(application).map_err(BootstrapError::Directories)?;
    let config_path = options
        .config()
        .map(std::path::Path::to_path_buf)
        .or_else(|| environment.config.as_ref().map(std::path::PathBuf::from))
        .unwrap_or_else(|| directories.config_path());
    let profiles = effective_profiles(&options, environment.profiles.as_deref());
    let config = if config_path.is_file() || config_path.extension().is_some() {
        ConfigManager::<Dynamic>::load_file(&config_path, &profiles)?
    } else {
        ConfigManager::<Dynamic>::load_in_explicit(&config_path, &profiles)?
    }
    .with_directories(&directories)
    .auto_discover()
    .with_config::<LoggingConfig>("logging");
    let mut logging = config.get_config::<LoggingConfig>("logging")?;

    if let Some(filter) = options.log().or(environment.rust_log.as_deref()) {
        logging.level = filter.to_owned();
    }

    if let Some(format) = options.log_format() {
        logging.format = format;
    } else if let Some(format) = environment.log_format.as_deref() {
        logging.format = parse_log_format(format)?;
    }

    let color = effective_color(options.color(), &environment);

    match color {
        ColorChoice::Always => logging.ansi = true,
        ColorChoice::Never => logging.ansi = false,
        ColorChoice::Auto => {}
    }

    let tracing_installed = if mode.is_run() {
        #[cfg(feature = "tracing-subscriber")]
        {
            crate::builtins::logging::init_tracing_resolved(&logging)?;

            true
        }

        #[cfg(not(feature = "tracing-subscriber"))]
        false
    } else {
        false
    };

    let state = BootstrapState {
        options,
        config_path,
        profiles,
        directories,
        config: Some(config),
        logging,
        color,
        tracing_installed,
    };
    let mut context = BootstrapContext::new(mode);

    context.set_bootstrap(state);

    Ok(context)
}

#[cfg(feature = "cli")]
fn effective_profiles(options: &BootstrapOptions, environment: Option<&str>) -> Vec<String> {
    if !options.profiles().is_empty() {
        return options.profiles().to_vec();
    }

    environment
        .into_iter()
        .flat_map(|profiles| profiles.split(','))
        .map(str::trim)
        .filter(|profile| !profile.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(feature = "cli")]
fn effective_color(cli: Option<ColorChoice>, environment: &BootstrapEnvironment) -> ColorChoice {
    if let Some(color) = cli {
        return color;
    }

    if environment.no_color {
        return ColorChoice::Never;
    }

    if environment
        .color_force
        .as_deref()
        .is_some_and(|value| value != "0")
    {
        return ColorChoice::Always;
    }

    ColorChoice::Auto
}

#[cfg(feature = "cli")]
fn parse_log_format(value: &str) -> Result<LogFormat, BootstrapError> {
    match value {
        "full" => Ok(LogFormat::Full),
        "compact" => Ok(LogFormat::Compact),
        "pretty" => Ok(LogFormat::Pretty),
        "json" => Ok(LogFormat::Json),
        _ => Err(BootstrapError::LogFormat {
            value: value.to_owned(),
        }),
    }
}

/// Protocol-neutral options consumed during generated application bootstrap.
#[cfg(feature = "cli")]
#[derive(Clone, Debug, Default, Eq, PartialEq, clap::Args)]
#[non_exhaustive]
pub struct BootstrapOptions {
    /// Configuration file or directory.
    #[arg(short = 'c', long, global = true, value_name = "PATH")]
    config: Option<std::path::PathBuf>,

    /// Ordered configuration profile; may be repeated.
    #[arg(short = 'p', long = "profile", global = true, value_name = "PROFILE")]
    profiles: Vec<String>,

    /// EnvFilter-compatible tracing directive.
    #[arg(long, global = true, value_name = "FILTER")]
    log: Option<String>,

    /// Tracing output formatter.
    #[arg(long, global = true, value_enum, value_name = "FORMAT")]
    log_format: Option<LogFormat>,

    /// ANSI color behavior.
    #[arg(long, global = true, value_enum, value_name = "WHEN")]
    color: Option<ColorChoice>,
}

#[cfg(feature = "cli")]
impl BootstrapOptions {
    /// Explicit configuration file or directory.
    pub fn config(&self) -> Option<&std::path::Path> {
        self.config.as_deref()
    }

    /// Ordered profiles selected on the command line.
    pub fn profiles(&self) -> &[String] {
        &self.profiles
    }

    /// Explicit tracing filter override.
    pub fn log(&self) -> Option<&str> {
        self.log.as_deref()
    }

    /// Explicit tracing formatter override.
    pub fn log_format(&self) -> Option<LogFormat> {
        self.log_format
    }

    /// Explicit color behavior override.
    pub fn color(&self) -> Option<ColorChoice> {
        self.color
    }
}

/// Failures returned by generated CLI parsing and dispatch.
#[cfg(feature = "cli")]
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CliError {
    /// Command-line arguments were invalid or requested early output such as help/version.
    #[error(transparent)]
    Clap(#[from] clap::Error),

    /// Framework bootstrap failed before the app lifecycle began.
    #[error(transparent)]
    Bootstrap(#[from] BootstrapError),

    /// Application bootstrap or lifecycle dispatch failed.
    #[error(transparent)]
    Lifecycle(#[from] PhaseError),
}

impl PhaseError {
    /// Wraps a typed lifecycle error with its phase.
    pub fn new(
        phase: LifecyclePhase,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            phase,
            source: Box::new(source),
        }
    }

    /// The phase that failed.
    pub fn phase(&self) -> LifecyclePhase {
        self.phase
    }
}

/// Static builder contract implemented by every generated named application host.
pub trait AppHost {
    /// The single protocol plugin configured by this host.
    type Protocol: ProtocolPlugin;

    /// Creates a fresh protocol-specific application builder.
    fn builder() -> Result<AppBuilder<Self::Protocol>, overseerd_config::ConfigError>;
}

#[cfg(test)]
mod tests;
