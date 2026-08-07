use std::ffi::OsString;
use std::path::PathBuf;

use cargo_upwell::{
    CargoExecutable, CommandKind, DiscoveryRequest, FeatureSelection, GraphQuery, InitRequest,
    TemplateSelection,
};
use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{Args, ColorChoice, CommandFactory as _, FromArgMatches as _, Parser, Subcommand};

mod format;

pub(crate) use format::{
    ExplainFormat, ExportFormat, GraphFormat, InspectCliProviderKind, InspectFormat,
    InspectResourceKind, ReportFormat, TerminalPolicy,
};
use format::{GraphFamily, GraphTraversalDirection};

const CARGO_HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::BrightGreen.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::BrightGreen.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::BrightCyan.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::BrightCyan.on_default());

/// Parsed `cargo upwell` process arguments.
#[derive(Debug, Parser)]
#[command(
    name = "cargo upwell",
    bin_name = "cargo upwell",
    version,
    about,
    styles = CARGO_HELP_STYLES,
    color = ColorChoice::Auto
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Available Cargo Upwell commands.
#[derive(Debug, Subcommand)]
enum Command {
    /// Generates an Upwell application, plugin, or protocol project.
    Init(InitArgs),
    /// Lists built-in and user-configured project templates.
    Templates(TemplatesArgs),
    /// Builds and validates one selected application without serving it.
    Check(ReportArgs),
    /// Diagnoses Cargo selection, build, probe, and application preparation.
    Doctor(ReportArgs),
    /// Displays the selected application's prepared tooling document.
    Inspect(InspectArgs),
    /// Emits the canonical tooling document or probe envelope.
    Export(ExportArgs),
    /// Renders application resources and their relationships.
    Graph(GraphArgs),
    /// Explains one exact resource identity or unique name.
    Explain(ExplainArgs),
}

/// Arguments for effective catalog template listing.
#[derive(Clone, Debug, Args)]
struct TemplatesArgs {
    /// Explicit Upwell catalog file.
    #[arg(long)]
    catalog: Option<PathBuf>,
}

/// Arguments for catalog-backed cargo-generate scaffolding.
#[derive(Clone, Debug, Args)]
struct InitArgs {
    /// Exact project directory to populate.
    path: PathBuf,
    /// Cargo package/project name. Defaults to the destination directory name.
    #[arg(long)]
    name: Option<String>,
    /// Catalog template ID.
    #[arg(long, conflicts_with = "template_path")]
    template: Option<String>,
    /// Direct local cargo-generate template directory.
    #[arg(long, conflicts_with_all = ["template", "catalog"])]
    template_path: Option<PathBuf>,
    /// Explicit Upwell catalog file.
    #[arg(long)]
    catalog: Option<PathBuf>,
    /// Add the generated crate to an immediate parent Cargo workspace.
    #[arg(long)]
    workspace: bool,
    /// Skip creation of a Git repository.
    #[arg(long)]
    no_vcs: bool,
    /// Supply a cargo-generate template value as key=value.
    #[arg(short = 'd', long = "define", action = clap::ArgAction::Append)]
    define: Vec<String>,
    /// Use a local Upwell repository dependency in built-in templates.
    #[arg(long)]
    upwell_path: Option<PathBuf>,
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

/// Arguments for deterministic graph rendering.
#[derive(Clone, Debug, Args)]
struct GraphArgs {
    #[command(flatten)]
    target: TargetArgs,
    /// Output representation.
    #[arg(long, value_enum, default_value_t = GraphFormat::Text)]
    format: GraphFormat,
    /// Semantic relationship family to include.
    #[arg(long, value_enum, default_value_t = GraphFamily::All)]
    family: GraphFamily,
    /// Semantic traversal direction from selected roots.
    #[arg(long, value_enum, default_value_t = GraphTraversalDirection::Both)]
    direction: GraphTraversalDirection,
    /// Select an exact stable resource ID or exact unique name.
    #[arg(long = "resource", action = clap::ArgAction::Append)]
    resources: Vec<String>,
    /// Select an exact contributor stable ID or exact unique name.
    #[arg(long = "contributor", action = clap::ArgAction::Append)]
    contributors: Vec<String>,
    /// Select an exact plugin stable ID or exact unique name.
    #[arg(long = "plugin", action = clap::ArgAction::Append)]
    plugins: Vec<String>,
    #[command(flatten)]
    terminal: TerminalArgs,
}

/// Arguments for one resource explanation.
#[derive(Clone, Debug, Args)]
struct ExplainArgs {
    /// Exact stable resource ID or exact unique name.
    resource: String,
    #[command(flatten)]
    target: TargetArgs,
    /// Output representation.
    #[arg(long, value_enum, default_value_t = ExplainFormat::Text)]
    format: ExplainFormat,
    #[command(flatten)]
    terminal: TerminalArgs,
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

/// Parsed Cargo Upwell command request.
#[derive(Debug)]
pub(crate) enum CommandRequest {
    /// Generate a project from a catalog or direct local template.
    Init(InitRequest),
    /// List effective project templates.
    Templates {
        /// Explicit catalog file.
        catalog_path: Option<PathBuf>,
    },
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
    /// Deterministic graph projection.
    Graph {
        discovery: DiscoveryRequest,
        format: GraphFormat,
        query: GraphQuery,
        color: TerminalPolicy,
        pager: TerminalPolicy,
    },
    /// One deterministic resource explanation.
    Explain {
        discovery: DiscoveryRequest,
        format: ExplainFormat,
        resource: String,
        color: TerminalPolicy,
        pager: TerminalPolicy,
    },
}

impl Cli {
    pub(crate) fn parse_cargo() -> Self {
        let arguments = normalized_arguments(std::env::args_os());
        let mut matches = Self::command()
            .color(help_color_policy())
            .get_matches_from(arguments);

        Self::from_arg_matches_mut(&mut matches).expect("Clap arguments match the derived CLI")
    }

    pub(crate) fn into_request(self) -> CommandRequest {
        match self.command {
            Command::Init(arguments) => {
                let template = arguments.template_path.map_or_else(
                    || TemplateSelection::Catalog {
                        template: arguments.template,
                        catalog_path: arguments.catalog,
                    },
                    TemplateSelection::Local,
                );

                CommandRequest::Init(InitRequest {
                    destination: arguments.path,
                    name: arguments.name,
                    template,
                    workspace: arguments.workspace,
                    no_vcs: arguments.no_vcs,
                    define: arguments.define,
                    upwell_path: arguments.upwell_path,
                })
            }
            Command::Templates(arguments) => CommandRequest::Templates {
                catalog_path: arguments.catalog,
            },
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
            Command::Graph(arguments) => CommandRequest::Graph {
                discovery: discovery_request(arguments.target),
                format: arguments.format,
                query: GraphQuery {
                    resources: arguments.resources,
                    contributors: arguments.contributors,
                    plugins: arguments.plugins,
                    family: arguments.family.into(),
                    direction: arguments.direction.into(),
                },
                color: arguments.terminal.color,
                pager: arguments.terminal.pager,
            },
            Command::Explain(arguments) => CommandRequest::Explain {
                discovery: discovery_request(arguments.target),
                format: arguments.format,
                resource: arguments.resource,
                color: arguments.terminal.color,
                pager: arguments.terminal.pager,
            },
        }
    }
}

fn help_color_policy() -> ColorChoice {
    match std::env::var("CARGO_TERM_COLOR").as_deref() {
        Ok("always") => ColorChoice::Always,
        Ok("never") => ColorChoice::Never,
        _ if std::env::var_os("NO_COLOR").is_some() => ColorChoice::Never,
        _ if std::env::var_os("TERM").is_some_and(|value| value == "dumb") => ColorChoice::Never,
        _ => ColorChoice::Auto,
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
        .is_some_and(|argument| argument == "upwell")
    {
        arguments.remove(1);
    }

    arguments
}

#[cfg(test)]
mod tests;
