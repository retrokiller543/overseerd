use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;

use cargo_metadata::{CompilerMessage, Message};
use thiserror::Error;

use crate::process::{CARGO_STDERR_LIMIT, CARGO_STDOUT_LIMIT, ProcessExecutionError, execute};
use crate::{CancellationToken, CargoExecutable, FeatureSelection, ProcessStatus, SelectedTarget};

/// One structured rustc diagnostic emitted by Cargo while building the selected target.
#[derive(Clone, Debug)]
pub struct CargoDiagnostic {
    /// Cargo package identifier associated with the diagnostic.
    pub package_id: String,
    /// Cargo target name associated with the diagnostic.
    pub target_name: String,
    /// Complete rustc diagnostic, including source spans and suggestions.
    pub diagnostic: cargo_metadata::diagnostic::Diagnostic,
}

impl From<CompilerMessage> for CargoDiagnostic {
    fn from(message: CompilerMessage) -> Self {
        Self {
            package_id: message.package_id.to_string(),
            target_name: message.target.name,
            diagnostic: message.message,
        }
    }
}

/// Inputs for building one selected Cargo binary target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildRequest {
    /// Cargo executable used for the build.
    pub cargo: CargoExecutable,
    /// Working directory inherited from the invoking terminal or editor.
    pub current_dir: Option<PathBuf>,
    /// Optional explicit target triple.
    pub target: Option<String>,
    /// Selected package feature settings.
    pub features: FeatureSelection,
    /// Effective Cargo target directory reported by metadata.
    pub workspace_target_directory: PathBuf,
    /// Explicitly selected package and binary target.
    pub selected: SelectedTarget,
}

/// Structured evidence emitted by Cargo while building the selected target.
#[derive(Debug)]
pub struct BuildEvidence {
    /// Portable process completion status.
    pub status: ProcessStatus,
    /// Source-preserving rustc diagnostics for editor integrations.
    pub diagnostics: Vec<CargoDiagnostic>,
    /// Non-JSON Cargo output lines retained without interpretation.
    pub text_lines: Vec<String>,
    /// Cargo stderr retained independently from diagnostics.
    pub stderr: Vec<u8>,
    /// Whether Cargo stdout exceeded the retained machine-output bound.
    pub stdout_truncated: bool,
    /// Whether Cargo stderr exceeded the retained bound.
    pub stderr_truncated: bool,
}

/// Successful selected-target build result.
#[derive(Debug)]
pub struct BuildResult {
    /// Exact executable path emitted by Cargo.
    pub executable: PathBuf,
    /// Absolute tooling-owned Cargo target directory.
    pub target_directory: PathBuf,
    /// Cargo build evidence available to terminal and IDE consumers.
    pub evidence: BuildEvidence,
}

/// Failure while building or resolving the selected executable artifact.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BuildError {
    /// Cancellation was requested while Cargo was running.
    #[error("Cargo build was cancelled")]
    Cancelled {
        /// Cargo output emitted before cancellation completed.
        evidence: BuildEvidence,
    },
    /// Cargo could not be launched.
    #[error("failed to launch Cargo build")]
    Launch(#[source] std::io::Error),
    /// Cargo process monitoring failed.
    #[error("failed while waiting for the Cargo build")]
    Process(#[source] std::io::Error),
    /// A Cargo output reader thread failed unexpectedly.
    #[error("failed to capture Cargo build output")]
    Capture,
    /// Cargo emitted an invalid machine-message stream.
    #[error("failed to parse Cargo build messages")]
    Message {
        /// Cargo output and source diagnostics decoded before the invalid line.
        evidence: BuildEvidence,
        /// Message stream I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// Cargo machine output exceeded the supported retained size.
    #[error("Cargo build output exceeded the supported size")]
    OutputTooLarge {
        /// Bounded Cargo output and source diagnostics.
        evidence: BuildEvidence,
    },
    /// Cargo reported a build failure.
    #[error("Cargo failed to build the selected application target")]
    Failed {
        /// Cargo output and source diagnostics.
        evidence: BuildEvidence,
    },
    /// Cargo completed successfully without its terminal build-finished message.
    #[error("Cargo did not report a completed build")]
    MissingBuildFinished {
        /// Cargo output and source diagnostics.
        evidence: BuildEvidence,
    },
    /// Cargo did not emit an executable artifact for the selected binary.
    #[error("Cargo emitted no executable for the selected application target")]
    MissingExecutable {
        /// Cargo output and source diagnostics.
        evidence: BuildEvidence,
    },
    /// Cargo emitted multiple different executables for one selected target.
    #[error("Cargo emitted multiple executables for the selected application target")]
    AmbiguousExecutable {
        /// Distinct executable paths in stable order.
        executables: Vec<PathBuf>,
        /// Cargo output and source diagnostics.
        evidence: BuildEvidence,
    },
    /// Cargo's selected executable path is not a regular file.
    #[error("Cargo's selected executable artifact is not a regular file")]
    InvalidExecutable {
        /// Cargo-emitted executable path.
        path: PathBuf,
        /// Cargo output and source diagnostics.
        evidence: BuildEvidence,
    },
}

/// Builds one selected binary in the tooling-owned Cargo target directory.
pub fn build_target(
    request: &BuildRequest,
    cancellation: &CancellationToken,
) -> Result<BuildResult, BuildError> {
    let target_directory = request.workspace_target_directory.join("overseerd/build");
    let mut command = Command::new(request.cargo.as_os_str());
    let mut diagnostics = Vec::new();
    let mut text_lines = Vec::new();
    let mut executables = Vec::new();
    let mut build_finished = None;

    command
        .arg("build")
        .arg("--manifest-path")
        .arg(&request.selected.manifest_path)
        .arg("--package")
        .arg(&request.selected.package_id)
        .arg("--bin")
        .arg(&request.selected.binary_name)
        .arg("--message-format=json-render-diagnostics")
        .env("CARGO_TARGET_DIR", &target_directory);

    apply_feature_arguments(&mut command, &request.features);

    if let Some(target) = &request.target {
        command.arg("--target").arg(target);
    }

    if let Some(current_dir) = &request.current_dir {
        command.current_dir(current_dir);
    }

    let output = execute(
        &mut command,
        cancellation,
        CARGO_STDOUT_LIMIT,
        CARGO_STDERR_LIMIT,
    )
    .map_err(map_process_error)?;

    for message in Message::parse_stream(Cursor::new(&output.stdout)) {
        let message = match message {
            Ok(message) => message,
            Err(source) => {
                let evidence = BuildEvidence {
                    status: output.status,
                    diagnostics,
                    text_lines,
                    stderr: output.stderr,
                    stdout_truncated: output.stdout_truncated,
                    stderr_truncated: output.stderr_truncated,
                };

                if output.cancelled {
                    return Err(BuildError::Cancelled { evidence });
                }

                return Err(BuildError::Message { evidence, source });
            }
        };

        match message {
            Message::CompilerArtifact(artifact)
                if artifact.package_id.to_string() == request.selected.package_id
                    && artifact.target.is_bin()
                    && artifact.target.name == request.selected.binary_name
                    && !artifact.profile.test =>
            {
                if let Some(executable) = artifact.executable {
                    executables.push(executable.into_std_path_buf());
                }
            }
            Message::CompilerMessage(message) => diagnostics.push(message.into()),
            Message::BuildFinished(finished) => build_finished = Some(finished.success),
            Message::TextLine(line) => text_lines.push(line),
            _ => {}
        }
    }

    executables.sort();
    executables.dedup();

    let evidence = BuildEvidence {
        status: output.status,
        diagnostics,
        text_lines,
        stderr: output.stderr,
        stdout_truncated: output.stdout_truncated,
        stderr_truncated: output.stderr_truncated,
    };

    if output.cancelled {
        return Err(BuildError::Cancelled { evidence });
    }

    if output.stdout_truncated {
        return Err(BuildError::OutputTooLarge { evidence });
    }

    if !output.status.success || build_finished == Some(false) {
        return Err(BuildError::Failed { evidence });
    }

    if build_finished != Some(true) {
        return Err(BuildError::MissingBuildFinished { evidence });
    }

    let executable = match executables.as_slice() {
        [] => return Err(BuildError::MissingExecutable { evidence }),
        [executable] => executable.clone(),
        _ => {
            return Err(BuildError::AmbiguousExecutable {
                executables,
                evidence,
            });
        }
    };

    if !executable.is_file() {
        return Err(BuildError::InvalidExecutable {
            path: executable,
            evidence,
        });
    }

    Ok(BuildResult {
        executable,
        target_directory,
        evidence,
    })
}

fn apply_feature_arguments(command: &mut Command, features: &FeatureSelection) {
    let normalized = features.normalized_features();

    if features.no_default_features {
        command.arg("--no-default-features");
    }

    if features.all_features {
        command.arg("--all-features");
    }

    if !normalized.is_empty() {
        command.arg("--features").arg(normalized.join(","));
    }
}

fn map_process_error(error: ProcessExecutionError) -> BuildError {
    match error {
        ProcessExecutionError::Spawn(source) => BuildError::Launch(source),
        ProcessExecutionError::Wait(source)
        | ProcessExecutionError::Kill(source)
        | ProcessExecutionError::Capture(source) => BuildError::Process(source),
        ProcessExecutionError::CapturePanic => BuildError::Capture,
    }
}
