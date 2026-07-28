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
