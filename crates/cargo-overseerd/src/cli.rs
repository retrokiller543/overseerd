use std::ffi::OsString;
use std::path::PathBuf;

use cargo_overseerd::{CargoExecutable, CommandKind, DiscoveryRequest, FeatureSelection};
use clap::{Args, Parser, Subcommand, ValueEnum};

/// Parsed `cargo overseerd` process arguments.
#[derive(Debug, Parser)]
#[command(name = "cargo overseerd", version, about)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Available Cargo Overseerd commands.
#[derive(Debug, Subcommand)]
enum Command {
    /// Builds and validates one selected application without serving it.
    Check(ReportArgs),
    /// Diagnoses Cargo selection, build, probe, and application preparation.
    Doctor(ReportArgs),
    /// Displays the selected application's prepared tooling document.
    Inspect(InspectArgs),
    /// Emits the canonical tooling document or probe envelope.
    Export(ExportArgs),
}

/// Cargo target selection shared by application commands.
#[derive(Clone, Debug, Args)]
struct TargetArgs {
    /// Path to Cargo.toml.
    #[arg(long)]
    manifest_path: Option<PathBuf>,
    /// Workspace package containing the application.
    #[arg(short = 'p', long)]
    package: Option<String>,
    /// Binary target containing the application.
    #[arg(long = "bin")]
    binary: Option<String>,
    /// Package features enabled for discovery and build.
    #[arg(long, value_delimiter = ',', action = clap::ArgAction::Append)]
    features: Vec<String>,
    /// Enable every package feature.
    #[arg(long, conflicts_with = "features")]
    all_features: bool,
    /// Disable package default features.
    #[arg(long)]
    no_default_features: bool,
    /// Cargo build target triple.
    #[arg(long)]
    target: Option<String>,
}

/// Arguments for check and doctor reports.
#[derive(Clone, Debug, Args)]
struct ReportArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// Output representation.
    #[arg(long, value_enum, default_value_t = ReportFormat::Terminal)]
    format: ReportFormat,
}

/// Arguments for generic application inspection.
#[derive(Clone, Debug, Args)]
struct InspectArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// Output representation.
    #[arg(long, value_enum, default_value_t = InspectFormat::Text)]
    format: InspectFormat,
    #[command(flatten)]
    filters: InspectFilters,
    #[command(flatten)]
    terminal: TerminalArgs,
}

/// Arguments for canonical machine-readable export.
#[derive(Clone, Debug, Args)]
struct ExportArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// Canonical payload to emit.
    #[arg(long, value_enum, default_value_t = ExportFormat::Document)]
    format: ExportFormat,
    /// Write to a file instead of stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

/// Generic inspection filters combined across dimensions.
#[derive(Clone, Debug, Default, Args)]
pub(crate) struct InspectFilters {
    /// Include these generic resource kinds.
    #[arg(long = "kind", value_enum, value_delimiter = ',', action = clap::ArgAction::Append)]
    pub(crate) kinds: Vec<InspectResourceKind>,
    /// Include resources with these exact stable IDs or names.
    #[arg(long = "resource", value_delimiter = ',', action = clap::ArgAction::Append)]
    pub(crate) resources: Vec<String>,
    /// Include resources owned by these contributor IDs or names.
    #[arg(long = "contributor", value_delimiter = ',', action = clap::ArgAction::Append)]
    pub(crate) contributors: Vec<String>,
    /// Include these plugin IDs or names and their owned resources.
    #[arg(long = "plugin", value_delimiter = ',', action = clap::ArgAction::Append)]
    pub(crate) plugins: Vec<String>,
    /// Include resources carrying these facet namespaces.
    #[arg(long = "facet", value_delimiter = ',', action = clap::ArgAction::Append)]
    pub(crate) facets: Vec<String>,
    /// Include these scope IDs or names and resources assigned to them.
    #[arg(long = "scope", value_delimiter = ',', action = clap::ArgAction::Append)]
    pub(crate) scopes: Vec<String>,
    /// Include these plugin CLI provider categories.
    #[arg(long = "cli-provider-kind", value_enum, value_delimiter = ',', action = clap::ArgAction::Append)]
    pub(crate) cli_provider_kinds: Vec<InspectCliProviderKind>,
}

impl InspectFilters {
    pub(crate) fn is_empty(&self) -> bool {
        self.kinds.is_empty()
            && self.resources.is_empty()
            && self.contributors.is_empty()
            && self.plugins.is_empty()
            && self.facets.is_empty()
            && self.scopes.is_empty()
            && self.cli_provider_kinds.is_empty()
    }
}

/// Terminal presentation policy for human-readable inspection.
#[derive(Clone, Copy, Debug, Args)]
struct TerminalArgs {
    /// ANSI color policy.
    #[arg(long, value_enum, default_value_t = TerminalPolicy::Auto)]
    color: TerminalPolicy,
    /// Pager policy.
    #[arg(long, value_enum, default_value_t = TerminalPolicy::Auto)]
    pager: TerminalPolicy,
}

/// Parsed Cargo Overseerd command request.
#[derive(Debug)]
pub(crate) enum CommandRequest {
    /// Check or doctor report.
    Report {
        command: CommandKind,
        discovery: DiscoveryRequest,
        format: ReportFormat,
    },
    /// Human or canonical JSON inspection.
    Inspect {
        discovery: DiscoveryRequest,
        format: InspectFormat,
        filters: InspectFilters,
        color: TerminalPolicy,
        pager: TerminalPolicy,
    },
    /// Canonical document or envelope export.
    Export {
        discovery: DiscoveryRequest,
        format: ExportFormat,
        output: Option<PathBuf>,
    },
}

/// Supported check and doctor output representations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum ReportFormat {
    /// Human-readable terminal output.
    #[default]
    Terminal,
    /// Versioned machine-readable JSON.
    Json,
}

/// Supported inspection output representations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum InspectFormat {
    /// Human-readable generic inspection.
    #[default]
    Text,
    /// Canonical tooling document JSON.
    Json,
}

/// Canonical export payload.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum ExportFormat {
    /// Successful canonical tooling document.
    #[default]
    Document,
    /// Complete canonical success or failure probe envelope.
    Envelope,
}

/// Automatic, forced, or disabled terminal behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum TerminalPolicy {
    /// Enable behavior only for an interactive terminal.
    #[default]
    Auto,
    /// Always enable behavior.
    Always,
    /// Never enable behavior.
    Never,
}

/// Generic resource kind accepted by inspection filters.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum InspectResourceKind {
    Application,
    Protocol,
    Plugin,
    Component,
    Provider,
    ConfigBinding,
    Hook,
    Lifecycle,
    Scope,
    Type,
    Contribution,
    Contributor,
    PluginSlot,
}

/// CLI provider kind accepted by inspection filters.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum InspectCliProviderKind {
    Args,
    Command,
    CommandSet,
}

impl Cli {
    pub(crate) fn parse_cargo() -> Self {
        Self::parse_from(normalized_arguments(std::env::args_os()))
    }

    pub(crate) fn into_request(self) -> CommandRequest {
        match self.command {
            Command::Check(arguments) => CommandRequest::Report {
                command: CommandKind::Check,
                discovery: discovery_request(arguments.target),
                format: arguments.format,
            },
            Command::Doctor(arguments) => CommandRequest::Report {
                command: CommandKind::Doctor,
                discovery: discovery_request(arguments.target),
                format: arguments.format,
            },
            Command::Inspect(arguments) => CommandRequest::Inspect {
                discovery: discovery_request(arguments.target),
                format: arguments.format,
                filters: arguments.filters,
                color: arguments.terminal.color,
                pager: arguments.terminal.pager,
            },
            Command::Export(arguments) => CommandRequest::Export {
                discovery: discovery_request(arguments.target),
                format: arguments.format,
                output: arguments.output,
            },
        }
    }
}

fn discovery_request(arguments: TargetArgs) -> DiscoveryRequest {
    let current_dir = std::env::current_dir().ok();

    DiscoveryRequest {
        cargo: CargoExecutable::from_environment(),
        manifest_path: arguments.manifest_path,
        current_dir,
        package: arguments.package,
        binary: arguments.binary,
        features: FeatureSelection {
            no_default_features: arguments.no_default_features,
            all_features: arguments.all_features,
            features: arguments.features,
        },
        target: arguments.target,
    }
}

fn normalized_arguments(arguments: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut arguments = arguments.into_iter().collect::<Vec<_>>();

    if arguments
        .get(1)
        .is_some_and(|argument| argument == "overseerd")
    {
        arguments.remove(1);
    }

    arguments
}

#[cfg(test)]
mod tests;
