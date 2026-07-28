use cargo_overseerd::{
    CommandKind, CommandOutcome, CommandReport, CommandSchemaVersion, SelectedTargetReport,
};
use overseerd_tooling_schema::DocumentIdentity;

use super::write_report;
use crate::cli::OutputFormat;

#[test]
fn terminal_report_exposes_live_application_identity() {
    let report = CommandReport {
        schema: CommandSchemaVersion::CURRENT,
        command: CommandKind::Check,
        outcome: CommandOutcome::Success,
        exit_code: 0,
        target: Some(SelectedTargetReport {
            package: String::from("homeledger"),
            version: String::from("1.0.0"),
            binary: String::from("homeledger"),
            manifest_path: None,
        }),
        application: Some(DocumentIdentity {
            application: String::from("homeledger"),
            ..DocumentIdentity::default()
        }),
        protocol: Some(String::from("homeledger/rpc")),
        framework_version: Some(String::from("0.20.0")),
        checks: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut output = Vec::new();

    write_report(&report, OutputFormat::Terminal, &mut output).expect("terminal report writes");

    let output = String::from_utf8(output).expect("terminal report is UTF-8");

    assert!(output.contains("Application homeledger (homeledger/rpc)"));
    assert!(output.contains("cargo overseerd check passed"));
}

#[test]
fn json_report_is_one_machine_readable_line() {
    let report = CommandReport {
        schema: CommandSchemaVersion::CURRENT,
        command: CommandKind::Check,
        outcome: CommandOutcome::Success,
        exit_code: 0,
        target: None,
        application: None,
        protocol: None,
        framework_version: None,
        checks: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut output = Vec::new();

    write_report(&report, OutputFormat::Json, &mut output).expect("JSON report writes");

    let output = String::from_utf8(output).expect("JSON report is UTF-8");

    assert_eq!(output.lines().count(), 1);
    assert!(serde_json::from_str::<serde_json::Value>(output.trim()).is_ok());
}
