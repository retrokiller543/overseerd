use std::path::PathBuf;

use crate::{
    BuildError, BuildEvidence, ProbeError, ProbeEvidence, ProbeRequestError, ProcessStatus,
    SelectedTarget,
};
use overseerd_tooling_schema::{Diagnostic, DiagnosticSeverity};

use super::{CommandExitCode, CommandKind, CommandOutcome, CommandReport, CommandSchemaVersion};

#[test]
fn exit_categories_have_stable_distinct_codes() {
    assert_eq!(CommandExitCode::Success.code(), 0);
    assert_eq!(CommandExitCode::ValidationFailure.code(), 1);
    assert_eq!(CommandExitCode::Misuse.code(), 2);
    assert_eq!(CommandExitCode::TargetSelectionFailure.code(), 3);
    assert_eq!(CommandExitCode::BuildFailure.code(), 4);
    assert_eq!(CommandExitCode::ProbeFailure.code(), 5);
    assert_eq!(CommandExitCode::OperationalFailure.code(), 6);
}

#[test]
fn json_report_is_versioned_and_canonicalizes_diagnostics() {
    let mut report = CommandReport::new(CommandKind::Check, CommandOutcome::ValidationFailure);

    report.diagnostics = vec![
        Diagnostic {
            code: String::from("fixture/z-last"),
            severity: DiagnosticSeverity::Warning,
            message: String::from("last"),
            ..Diagnostic::default()
        },
        Diagnostic {
            code: String::from("fixture/a-first"),
            severity: DiagnosticSeverity::Error,
            message: String::from("first"),
            resources: vec![String::from("z"), String::from("a"), String::from("a")],
            ..Diagnostic::default()
        },
    ];
    report.canonicalize();

    let json = report.to_json().expect("command report serializes");

    assert_eq!(report.schema, CommandSchemaVersion::CURRENT);
    assert!(json.starts_with("{\"schema\":{\"major\":1},\"command\":\"check\""));
    assert!(json.find("fixture/a-first") < json.find("fixture/z-last"));
    assert_eq!(report.diagnostics[0].resources, ["a", "z"]);
}

#[test]
fn cancellation_is_always_operational_and_retains_selected_target() {
    let target = selected_target();
    let report = super::report_error(
        CommandKind::Check,
        ProbeRequestError::Build {
            target: Box::new(target.clone()),
            source: Box::new(BuildError::Cancelled {
                evidence: empty_build_evidence(),
            }),
        },
    );

    assert_eq!(report.outcome, CommandOutcome::OperationalFailure);
    assert_eq!(report.exit_code, CommandExitCode::OperationalFailure.code());
    assert_eq!(report.target, Some((&target).into()));
}

#[test]
fn local_probe_failures_are_operational_and_retain_selected_target() {
    let target = selected_target();
    let errors = [
        ProbeError::Cancelled {
            evidence: Box::new(empty_probe_evidence()),
        },
        ProbeError::Launch(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "fixture launch failure",
        )),
    ];

    for error in errors {
        let report = super::report_error(
            CommandKind::Check,
            ProbeRequestError::Probe {
                target: Box::new(target.clone()),
                source: Box::new(error),
            },
        );

        assert_eq!(report.outcome, CommandOutcome::OperationalFailure);
        assert_eq!(report.exit_code, CommandExitCode::OperationalFailure.code());
        assert_eq!(report.target, Some((&target).into()));
    }
}

fn selected_target() -> SelectedTarget {
    SelectedTarget {
        package_id: String::from("fixture 1.0.0 (path+file:///fixture)"),
        package_name: String::from("fixture"),
        package_version: String::from("1.0.0"),
        manifest_path: PathBuf::from("/fixture/Cargo.toml"),
        binary_name: String::from("fixture"),
        required_features: Vec::new(),
    }
}

fn empty_build_evidence() -> BuildEvidence {
    BuildEvidence {
        status: ProcessStatus {
            success: false,
            code: None,
        },
        diagnostics: Vec::new(),
        text_lines: Vec::new(),
        stderr: Vec::new(),
        stdout_truncated: false,
        stderr_truncated: false,
    }
}

fn empty_probe_evidence() -> ProbeEvidence {
    ProbeEvidence {
        status: ProcessStatus {
            success: false,
            code: None,
        },
        stdout: Vec::new(),
        stderr: Vec::new(),
        stdout_truncated: false,
        stderr_truncated: false,
    }
}
