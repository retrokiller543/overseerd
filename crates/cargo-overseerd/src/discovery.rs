use std::ffi::OsString;
use std::path::PathBuf;

use cargo_metadata::MetadataCommand;
use thiserror::Error;

use crate::{
    CancellationToken, FeatureSelection, SelectedTarget, SelectionError, WorkspaceCatalog,
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
    /// Cargo metadata could not be loaded or decoded.
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

    let metadata = command.exec().map_err(DiscoveryError::Metadata)?;

    if cancellation.is_cancelled() {
        return Err(DiscoveryError::Cancelled);
    }
    let catalog = WorkspaceCatalog::from_metadata(&metadata, &request.features);
    let selected = catalog.select(request.package.as_deref(), request.binary.as_deref())?;

    Ok((catalog, selected))
}
