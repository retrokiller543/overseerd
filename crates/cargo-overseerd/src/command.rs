use overseerd_tooling_schema::{Diagnostic, DiagnosticSeverity, DocumentIdentity, ProbeOutcome};
use serde::Serialize;

use crate::{
    CancellationToken, DiscoveryRequest, ProbeRequestError, SelectedTarget, ToolingProbe, run_probe,
};

mod diagnostic;

use diagnostic::{
    append_evidence_diagnostics, build_diagnostics, discovery_diagnostics, probe_diagnostics,
    tool_diagnostic,
};

const COMMAND_SCHEMA_MAJOR: u16 = 1;

/// Version of the machine-readable Cargo command report.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CommandSchemaVersion {
    /// Breaking-change boundary for command reports.
    pub major: u16,
}

impl CommandSchemaVersion {
    /// Current command report version.
    pub const CURRENT: Self = Self {
        major: COMMAND_SCHEMA_MAJOR,
    };
}

/// Cargo Overseerd command represented by a report.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandKind {
    /// Builds and validates one selected application.
    Check,
    /// Diagnoses the selected application's tooling setup.
    Doctor,
}

/// Stable process exit categories for Cargo Overseerd commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandExitCode {
    /// The command completed successfully.
    Success,
    /// Application preparation or validation failed.
    ValidationFailure,
    /// Command-line usage was invalid.
    Misuse,
    /// Cargo workspace discovery or target selection failed.
    TargetSelectionFailure,
    /// Cargo could not build or resolve the selected executable.
    BuildFailure,
    /// The selected executable did not implement the compatible probe contract.
    ProbeFailure,
    /// Tool operation or cancellation failed independently of the application.
    OperationalFailure,
}

impl CommandExitCode {
    /// Returns the stable numeric process exit code.
    pub const fn code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::ValidationFailure => 1,
            Self::Misuse => 2,
            Self::TargetSelectionFailure => 3,
            Self::BuildFailure => 4,
            Self::ProbeFailure => 5,
            Self::OperationalFailure => 6,
        }
    }
}

/// Stable command outcome independent of terminal presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandOutcome {
    /// All requested checks passed.
    Success,
    /// The application emitted validation diagnostics.
    ValidationFailure,
    /// Cargo workspace discovery or target selection failed.
    TargetSelectionFailure,
    /// Cargo could not build or resolve the selected executable.
    BuildFailure,
    /// The executable did not complete the generated probe contract.
    ProbeFailure,
    /// Tool operation or cancellation failed.
    OperationalFailure,
}

impl CommandOutcome {
    fn exit_code(self) -> CommandExitCode {
        match self {
            Self::Success => CommandExitCode::Success,
            Self::ValidationFailure => CommandExitCode::ValidationFailure,
            Self::TargetSelectionFailure => CommandExitCode::TargetSelectionFailure,
            Self::BuildFailure => CommandExitCode::BuildFailure,
            Self::ProbeFailure => CommandExitCode::ProbeFailure,
            Self::OperationalFailure => CommandExitCode::OperationalFailure,
        }
    }
}

/// Selected Cargo target identity safe for command output.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SelectedTargetReport {
    /// Cargo package name.
    pub package: String,
    /// Cargo package version.
    pub version: String,
    /// Cargo binary target name.
    pub binary: String,
    /// Absolute manifest path when it is Unicode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_path: Option<String>,
}

impl From<&SelectedTarget> for SelectedTargetReport {
    fn from(target: &SelectedTarget) -> Self {
        Self {
            package: target.package_name.clone(),
            version: target.package_version.clone(),
            binary: target.binary_name.clone(),
            manifest_path: target.manifest_path.to_str().map(ToString::to_string),
        }
    }
}

/// Status of one doctor health check.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandCheckStatus {
    /// The health check passed.
    Passed,
    /// The health check found a non-fatal concern.
    Warning,
    /// The health check failed.
    Failed,
}

/// One stable health check reported by `cargo overseerd doctor`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CommandCheck {
    /// Stable namespaced check code.
    pub code: String,
    /// Check result.
    pub status: CommandCheckStatus,
    /// Human-readable result without raw application values.
    pub message: String,
}

/// Versioned presentation-neutral result of one Cargo Overseerd command.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CommandReport {
    /// Command report compatibility version.
    pub schema: CommandSchemaVersion,
    /// Command that produced this report.
    pub command: CommandKind,
    /// Stable outcome category.
    pub outcome: CommandOutcome,
    /// Stable numeric process exit code.
    pub exit_code: u8,
    /// Selected Cargo target when discovery completed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<SelectedTargetReport>,
    /// Generated application identity when the probe responded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application: Option<DocumentIdentity>,
    /// Selected protocol identity when preparation completed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    /// Framework version reported by the selected application.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framework_version: Option<String>,
    /// Doctor-specific health checks in execution order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<CommandCheck>,
    /// Canonical structured diagnostics.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

impl CommandReport {
    /// Serializes one deterministic machine-readable command report.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Returns the stable process exit category.
    pub fn exit_code(&self) -> CommandExitCode {
        self.outcome.exit_code()
    }

    fn new(command: CommandKind, outcome: CommandOutcome) -> Self {
        Self {
            schema: CommandSchemaVersion::CURRENT,
            command,
            outcome,
            exit_code: outcome.exit_code().code(),
            target: None,
            application: None,
            protocol: None,
            framework_version: None,
            checks: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn canonicalize(&mut self) {
        for diagnostic in &mut self.diagnostics {
            diagnostic.resources.sort();
            diagnostic.resources.dedup();
            diagnostic.sources.sort();
            diagnostic.sources.dedup();
        }

        self.diagnostics.sort_by(|left, right| {
            (
                &left.code,
                &left.severity,
                &left.message,
                &left.resources,
                &left.sources,
                &left.fix,
            )
                .cmp(&(
                    &right.code,
                    &right.severity,
                    &right.message,
                    &right.resources,
                    &right.sources,
                    &right.fix,
                ))
        });
        self.diagnostics.dedup_by(|left, right| left == right);
    }
}

/// Runs one command through the shared Cargo discovery, build, and target-local probe path.
pub fn run_command(
    command: CommandKind,
    request: &DiscoveryRequest,
    cancellation: &CancellationToken,
) -> CommandReport {
    let mut report = match run_probe(request, cancellation) {
        Ok(probe) => report_probe(command, probe),
        Err(error) => report_error(command, error),
    };

    report.canonicalize();

    report
}

fn report_probe(command: CommandKind, probe: ToolingProbe) -> CommandReport {
    let mut report = CommandReport::new(command, CommandOutcome::Success);

    report.target = Some(SelectedTargetReport::from(&probe.target));
    report.application = Some(probe.probe.envelope.identity.clone());

    if command == CommandKind::Doctor {
        report.checks.extend([
            passed_check(
                "cargo-overseerd/workspace",
                "Cargo workspace metadata and target selection succeeded.",
            ),
            passed_check(
                "cargo-overseerd/build",
                "The selected application target built in the isolated tooling directory.",
            ),
            passed_check(
                "cargo-overseerd/probe",
                "The generated target-local tooling probe is compatible.",
            ),
        ]);
    }

    match probe.probe.envelope.outcome {
        ProbeOutcome::Success { document } => {
            report.protocol = Some(document.protocol.clone());
            report.framework_version = Some(document.framework_version.clone());
            report.diagnostics = document.diagnostics.clone();

            if !document.validation.valid {
                report.outcome = CommandOutcome::ValidationFailure;
            }

            if command == CommandKind::Doctor {
                add_framework_version_check(&mut report, &document.framework_version);

                let status = if document.validation.valid {
                    CommandCheckStatus::Passed
                } else {
                    CommandCheckStatus::Failed
                };

                report.checks.push(CommandCheck {
                    code: String::from("cargo-overseerd/application-validation"),
                    status,
                    message: if document.validation.valid {
                        String::from("Application setup, configuration, and preparation succeeded.")
                    } else {
                        String::from("Application preparation reported validation failures.")
                    },
                });
            }
        }
        ProbeOutcome::Failure { failure } => {
            report.outcome = CommandOutcome::ValidationFailure;
            report.diagnostics = failure.diagnostics;

            if command == CommandKind::Doctor {
                report.checks.push(CommandCheck {
                    code: String::from("cargo-overseerd/application-validation"),
                    status: CommandCheckStatus::Failed,
                    message: String::from(
                        "Application setup, configuration, or preparation failed.",
                    ),
                });
            }
        }
    }

    append_evidence_diagnostics(
        &mut report.diagnostics,
        &probe.build.evidence,
        &probe.probe.evidence,
        probe.probe.cleanup_error.as_ref(),
    );
    report.exit_code = report.outcome.exit_code().code();

    report
}

fn add_framework_version_check(report: &mut CommandReport, framework_version: &str) {
    if framework_version == env!("CARGO_PKG_VERSION") {
        report.checks.push(passed_check(
            "cargo-overseerd/framework-version",
            "Cargo Overseerd and the application framework use the same version.",
        ));

        return;
    }

    report.outcome = CommandOutcome::ValidationFailure;
    report.checks.push(CommandCheck {
        code: String::from("cargo-overseerd/framework-version"),
        status: CommandCheckStatus::Failed,
        message: String::from("Cargo Overseerd and the application framework versions differ."),
    });
    report.diagnostics.push(Diagnostic {
        code: String::from("cargo-overseerd/framework-version-mismatch"),
        severity: DiagnosticSeverity::Error,
        message: format!(
            "Cargo Overseerd {} cannot diagnose an application using Overseerd {framework_version}.",
            env!("CARGO_PKG_VERSION")
        ),
        resources: Vec::new(),
        sources: Vec::new(),
        fix: Some(String::from(
            "Install the cargo-overseerd version matching the application's Overseerd dependencies.",
        )),
    });
}

fn report_error(command: CommandKind, error: ProbeRequestError) -> CommandReport {
    let (outcome, diagnostics) = match error {
        ProbeRequestError::Discovery(error) => (
            CommandOutcome::TargetSelectionFailure,
            discovery_diagnostics(&error),
        ),
        ProbeRequestError::Build(error) => {
            (CommandOutcome::BuildFailure, build_diagnostics(&error))
        }
        ProbeRequestError::Probe(error) => {
            (CommandOutcome::ProbeFailure, probe_diagnostics(&error))
        }
        ProbeRequestError::Lock(error) => (
            CommandOutcome::OperationalFailure,
            vec![tool_diagnostic(
                "cargo-overseerd/invocation-lock",
                "Cargo Overseerd could not serialize access to its private artifacts.",
                Some(format!(
                    "Verify the Cargo target directory is writable: {error}"
                )),
            )],
        ),
        ProbeRequestError::LockCancelled => (
            CommandOutcome::OperationalFailure,
            vec![tool_diagnostic(
                "cargo-overseerd/cancelled",
                "The tooling command was cancelled while waiting for another invocation.",
                None,
            )],
        ),
    };
    let mut report = CommandReport::new(command, outcome);

    report.diagnostics = diagnostics;

    if command == CommandKind::Doctor {
        report.checks.push(CommandCheck {
            code: String::from("cargo-overseerd/command"),
            status: CommandCheckStatus::Failed,
            message: String::from("The diagnostic command could not reach application validation."),
        });
    }

    report
}

fn passed_check(code: &str, message: &str) -> CommandCheck {
    CommandCheck {
        code: code.to_string(),
        status: CommandCheckStatus::Passed,
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests;
