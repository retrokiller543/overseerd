//! Cargo target discovery and probe execution for Overseerd developer tooling.
//!
//! This crate contains presentation-neutral orchestration and command reports shared by the
//! `cargo overseerd` subcommand and editor integrations. Terminal rendering remains in the binary.

mod build;
mod command;
mod discovery;
mod probe;
mod process;
mod selection;

use std::fs::OpenOptions;

pub use build::{BuildError, BuildEvidence, BuildResult, CargoDiagnostic};
use build::{BuildRequest, build_target};
pub use command::{
    CommandCheck, CommandCheckStatus, CommandExitCode, CommandKind, CommandOutcome, CommandReport,
    CommandSchemaVersion, SelectedTargetReport, run_command,
};
pub use discovery::{CargoExecutable, DiscoveryError, DiscoveryRequest, discover};
use fs2::FileExt as _;
use probe::execute_probe;
pub use probe::{ProbeError, ProbeEvidence, ProbeResult};
pub use process::{CancellationToken, ProcessStatus};
pub use selection::{
    BinaryCandidate, FeatureSelection, PackageCandidate, SelectedTarget, SelectionError,
    WorkspaceCatalog,
};

use thiserror::Error;

/// Complete selected-target probe result returned without presentation policy.
#[derive(Debug)]
pub struct ToolingProbe {
    /// Cargo workspace facts used for selection and artifact isolation.
    pub workspace: WorkspaceCatalog,
    /// Selected package and binary identity.
    pub target: SelectedTarget,
    /// Cargo build result and diagnostics.
    pub build: BuildResult,
    /// Validated target-local probe response and process evidence.
    pub probe: ProbeResult,
}

/// Failure from the complete discover, build, and probe orchestration.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProbeRequestError {
    /// Cargo workspace discovery or target selection failed.
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    /// The selected target could not be built.
    #[error(transparent)]
    Build(#[from] BuildError),
    /// The selected executable did not complete the private probe contract.
    #[error("the selected executable did not complete the private tooling probe")]
    Probe(#[source] Box<ProbeError>),
    /// Tooling invocation serialization could not be established.
    #[error("failed to acquire the Cargo tooling invocation lock")]
    Lock(#[source] std::io::Error),
    /// Cancellation was requested while waiting for another tooling invocation.
    #[error("tooling probe was cancelled while waiting for the invocation lock")]
    LockCancelled,
}

/// Discovers, builds, and executes one selected Overseerd application probe.
pub fn run_probe(
    request: &DiscoveryRequest,
    cancellation: &CancellationToken,
) -> Result<ToolingProbe, ProbeRequestError> {
    let (workspace, _) = discover(request, cancellation)?;
    let _lock = InvocationLock::acquire(&workspace.target_directory, cancellation)?;
    let (workspace, target) = discover(request, cancellation)?;
    let build = build_target(
        &BuildRequest {
            cargo: request.cargo.clone(),
            current_dir: request.current_dir.clone(),
            target: request.target.clone(),
            features: request.features.clone(),
            workspace_target_directory: workspace.target_directory.clone(),
            selected: target.clone(),
        },
        cancellation,
    )?;
    let probe = execute_probe(
        &build.executable,
        &workspace.target_directory,
        &target,
        request.current_dir.as_deref(),
        cancellation,
    )
    .map_err(|error| ProbeRequestError::Probe(Box::new(error)))?;

    Ok(ToolingProbe {
        workspace,
        target,
        build,
        probe,
    })
}

struct InvocationLock {
    file: std::fs::File,
}

impl InvocationLock {
    fn acquire(
        workspace_target_directory: &std::path::Path,
        cancellation: &CancellationToken,
    ) -> Result<Self, ProbeRequestError> {
        let directory = workspace_target_directory.join("overseerd");
        let path = directory.join("invocation.lock");

        std::fs::create_dir_all(&directory).map_err(ProbeRequestError::Lock)?;

        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)
            .map_err(ProbeRequestError::Lock)?;

        loop {
            if cancellation.is_cancelled() {
                return Err(ProbeRequestError::LockCancelled);
            }

            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Self { file }),
                Err(error) if lock_is_contended(&error) => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(error) => return Err(ProbeRequestError::Lock(error)),
            }
        }
    }
}

fn lock_is_contended(error: &std::io::Error) -> bool {
    let contended = fs2::lock_contended_error();

    error.raw_os_error() == contended.raw_os_error()
}

impl Drop for InvocationLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
