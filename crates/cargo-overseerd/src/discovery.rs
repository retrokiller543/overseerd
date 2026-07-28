use std::ffi::OsString;
use std::path::PathBuf;

use cargo_metadata::MetadataCommand;
use thiserror::Error;

use crate::process::{CARGO_STDERR_LIMIT, CARGO_STDOUT_LIMIT, ProcessExecutionError, execute};
use crate::{
    CancellationToken, FeatureSelection, ProcessStatus, SelectedTarget, SelectionError,
    WorkspaceCatalog,
};

/// Cargo executable used for metadata and build operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CargoExecutable {
    path: OsString,
}

impl CargoExecutable {
    /// Uses one explicit Cargo executable path or command name.
    pub fn new(path: impl Into<OsString>) -> Self {
        Self { path: path.into() }
    }

    /// Resolves the Cargo executable supplied to external subcommands.
    pub fn from_environment() -> Self {
        let path = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));

        Self { path }
    }

    /// Returns the executable path or command name passed to process creation.
    pub fn as_os_str(&self) -> &std::ffi::OsStr {
        &self.path
    }
}

impl Default for CargoExecutable {
    fn default() -> Self {
        Self::from_environment()
    }
}

/// Inputs that identify one Cargo workspace package and binary target.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiscoveryRequest {
    /// Cargo executable used for metadata discovery.
    pub cargo: CargoExecutable,
    /// Manifest path accepted by Cargo, relative to `current_dir` when not absolute.
    pub manifest_path: Option<PathBuf>,
    /// Working directory inherited from the invoking terminal or editor.
    pub current_dir: Option<PathBuf>,
    /// Explicit workspace package name.
    pub package: Option<String>,
    /// Explicit binary target name.
    pub binary: Option<String>,
    /// Package feature settings used for target eligibility.
    pub features: FeatureSelection,
    /// Optional build target triple retained for probe construction.
    pub target: Option<String>,
}

/// Cargo metadata discovery or target selection failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DiscoveryError {
    /// Cancellation was requested while Cargo metadata was running.
    #[error("Cargo metadata discovery was cancelled")]
    Cancelled,
    /// Cargo metadata could not be launched.
    #[error("failed to launch Cargo metadata discovery")]
    Launch(#[source] std::io::Error),
    /// Cargo metadata process monitoring failed.
    #[error("failed while waiting for Cargo metadata discovery")]
    Process(#[source] std::io::Error),
    /// A Cargo output reader thread failed unexpectedly.
    #[error("failed to capture Cargo metadata output")]
    Capture,
    /// Cargo metadata reported a failure.
    #[error("Cargo failed to load workspace metadata")]
    Failed {
        /// Portable Cargo completion status.
        status: ProcessStatus,
        /// Bounded Cargo stderr.
        stderr: Vec<u8>,
        /// Whether stderr exceeded the retained bound.
        stderr_truncated: bool,
    },
    /// Cargo metadata exceeded the bounded machine-output size.
    #[error("Cargo metadata output exceeded the supported size")]
    OutputTooLarge,
    /// Cargo metadata output was not UTF-8 text.
    #[error("Cargo metadata output is not UTF-8")]
    Encoding(#[source] std::string::FromUtf8Error),
    /// Cargo metadata could not be decoded.
    #[error("failed to load Cargo workspace metadata")]
    Metadata(#[source] cargo_metadata::Error),
    /// No unambiguous package and binary target matched the request.
    #[error(transparent)]
    Selection(#[from] SelectionError),
}

/// Loads Cargo metadata and selects one application binary target.
pub fn discover(
    request: &DiscoveryRequest,
    cancellation: &CancellationToken,
) -> Result<(WorkspaceCatalog, SelectedTarget), DiscoveryError> {
    let mut command = MetadataCommand::new();

    if cancellation.is_cancelled() {
        return Err(DiscoveryError::Cancelled);
    }

    command.cargo_path(request.cargo.as_os_str());
    command.no_deps();

    if let Some(path) = &request.manifest_path {
        command.manifest_path(path);
    }

    if let Some(path) = &request.current_dir {
        command.current_dir(path);
    }

    let mut cargo = command.cargo_command();
    let output = execute(
        &mut cargo,
        cancellation,
        CARGO_STDOUT_LIMIT,
        CARGO_STDERR_LIMIT,
    )
    .map_err(map_process_error)?;

    if output.cancelled {
        return Err(DiscoveryError::Cancelled);
    }

    if !output.status.success {
        return Err(DiscoveryError::Failed {
            status: output.status,
            stderr: output.stderr,
            stderr_truncated: output.stderr_truncated,
        });
    }

    if output.stdout_truncated {
        return Err(DiscoveryError::OutputTooLarge);
    }

    let stdout = String::from_utf8(output.stdout).map_err(DiscoveryError::Encoding)?;
    let json = stdout
        .lines()
        .find(|line| line.starts_with('{'))
        .unwrap_or(stdout.as_str());
    let metadata = MetadataCommand::parse(json).map_err(DiscoveryError::Metadata)?;
    let catalog = WorkspaceCatalog::from_metadata(&metadata, &request.features);
    let selected = catalog.select(request.package.as_deref(), request.binary.as_deref())?;

    Ok((catalog, selected))
}

fn map_process_error(error: ProcessExecutionError) -> DiscoveryError {
    match error {
        ProcessExecutionError::Spawn(source) => DiscoveryError::Launch(source),
        ProcessExecutionError::Wait(source)
        | ProcessExecutionError::Kill(source)
        | ProcessExecutionError::Capture(source) => DiscoveryError::Process(source),
        ProcessExecutionError::CapturePanic => DiscoveryError::Capture,
    }
}
