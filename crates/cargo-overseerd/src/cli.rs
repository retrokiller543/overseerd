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
    Check(TargetArgs),
    /// Diagnoses Cargo selection, build, probe, and application preparation.
    Doctor(TargetArgs),
}

/// Cargo target selection and output arguments shared by application commands.
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
    /// Output representation.
    #[arg(long, value_enum, default_value_t = OutputFormat::Terminal)]
    format: OutputFormat,
}

/// Supported command output representations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum OutputFormat {
    /// Human-readable terminal output.
    #[default]
    Terminal,
    /// Versioned machine-readable JSON.
    Json,
}

impl Cli {
    pub(crate) fn parse_cargo() -> Self {
        Self::parse_from(normalized_arguments(std::env::args_os()))
    }

    pub(crate) fn into_request(self) -> (CommandKind, DiscoveryRequest, OutputFormat) {
        let (command, arguments) = match self.command {
            Command::Check(arguments) => (CommandKind::Check, arguments),
            Command::Doctor(arguments) => (CommandKind::Doctor, arguments),
        };
        let current_dir = std::env::current_dir().ok();
        let request = DiscoveryRequest {
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
        };

        (command, request, arguments.format)
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
