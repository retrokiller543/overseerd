use overseerd_config::{ConfigManager, Dynamic};
use overseerd_dirs::DirectoriesManager;

use super::super::{BootstrapContext, PhaseError};
use super::{CliDefinitionError, CommandContextError, CommandError};
use crate::{LogFormat, LoggingConfig};

/// Controls when generated CLI output uses ANSI color.
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
pub struct BootstrapState {
    pub(super) options: BootstrapOptions,
    pub(super) config_path: std::path::PathBuf,
    pub(super) profiles: Vec<String>,
    pub(super) directories: Option<DirectoriesManager>,
    pub(super) config: Option<ConfigManager<Dynamic>>,
    pub(super) logging: LoggingConfig,
    pub(super) color: ColorChoice,
    pub(super) tracing_installed: bool,
    #[cfg(feature = "tracing-subscriber")]
    pub(super) tracing_layers: Vec<crate::builtins::BoxedLayer>,
}

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
    pub fn directories(&self) -> Option<&DirectoriesManager> {
        self.directories.as_ref()
    }

    /// Moves the merged config manager into an application builder once.
    pub fn take_config(&mut self) -> Option<ConfigManager<Dynamic>> {
        self.config.take()
    }

    /// Adds a tracing layer before generated bootstrap installs the global subscriber.
    #[cfg(feature = "tracing-subscriber")]
    pub fn add_tracing_layer(&mut self, layer: crate::builtins::BoxedLayer) {
        self.tracing_layers.push(layer);
    }
}

impl BootstrapContext {
    /// Resolved framework bootstrap state, when generated CLI bootstrap has run.
    pub fn bootstrap(&self) -> Option<&BootstrapState> {
        self.get()
    }

    /// Mutable resolved framework bootstrap state.
    pub fn bootstrap_mut(&mut self) -> Option<&mut BootstrapState> {
        self.get_mut()
    }

    /// Replaces this context's generated CLI bootstrap state.
    pub fn set_bootstrap(&mut self, state: BootstrapState) {
        self.insert(state);
    }
}

/// Failures from generated framework bootstrap.
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
    /// An explicit config path did not identify an existing file or directory.
    #[error("explicit config path '{}' does not exist", .path.display())]
    MissingConfigPath { path: std::path::PathBuf },
    /// The generated bootstrap could not install tracing.
    #[cfg(feature = "tracing-subscriber")]
    #[error(transparent)]
    Tracing(#[from] crate::builtins::InitTracingError),
}

/// Selects which framework managers generated bootstrap owns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootstrapPolicy {
    pub(super) directories: bool,
    pub(super) config: bool,
}

impl BootstrapPolicy {
    /// Creates a policy from generated directory/config ownership flags.
    pub const fn new(directories: bool, config: bool) -> Self {
        Self {
            directories,
            config,
        }
    }
}

impl Default for BootstrapPolicy {
    fn default() -> Self {
        Self::new(true, true)
    }
}

/// Protocol-neutral options consumed during generated application bootstrap.
#[derive(Clone, Debug, Default, Eq, PartialEq, clap::Args)]
#[non_exhaustive]
pub struct BootstrapOptions {
    /// Configuration file or directory.
    #[arg(short = 'c', long, global = true, value_name = "PATH")]
    pub(super) config: Option<std::path::PathBuf>,
    /// Ordered configuration profile; may be repeated.
    #[arg(short = 'p', long = "profile", global = true, value_name = "PROFILE")]
    pub(super) profiles: Vec<String>,
    /// EnvFilter-compatible tracing directive.
    #[arg(long, global = true, value_name = "FILTER")]
    pub(super) log: Option<String>,
    /// Tracing output formatter.
    #[arg(long, global = true, value_enum, value_name = "FORMAT")]
    pub(super) log_format: Option<LogFormat>,
    /// ANSI color behavior.
    #[arg(long, global = true, value_enum, value_name = "WHEN")]
    pub(super) color: Option<ColorChoice>,
}

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
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CliError {
    /// Generated or flattened Clap declarations conflict structurally.
    #[error(transparent)]
    Definition(#[from] CliDefinitionError),
    /// Command-line arguments were invalid or requested early output such as help/version.
    #[error(transparent)]
    Clap(#[from] clap::Error),
    /// Rendering process-facing help or version output failed.
    #[error("failed to render command-line output: {0}")]
    Output(#[from] std::io::Error),
    /// Framework bootstrap failed before the app lifecycle began.
    #[error(transparent)]
    Bootstrap(#[from] BootstrapError),
    /// Application bootstrap or lifecycle dispatch failed.
    #[error(transparent)]
    Lifecycle(#[from] PhaseError),
    /// An application-defined command failed.
    #[error(transparent)]
    Command(#[from] CommandError),
    /// Generated command dispatch received inconsistent lifecycle state.
    #[error(transparent)]
    CommandContext(#[from] CommandContextError),
}
