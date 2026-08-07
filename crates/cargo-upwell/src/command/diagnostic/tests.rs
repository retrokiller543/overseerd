use crate::{BuildError, BuildEvidence, ProcessStatus};
use upwell_tooling_schema::DiagnosticSeverity;

#[test]
fn rustc_warning_keeps_its_severity_and_does_not_hide_build_failure() {
    let error = BuildError::Failed {
        evidence: BuildEvidence {
            status: ProcessStatus {
                success: false,
                code: Some(1),
            },
            diagnostics: vec![crate::CargoDiagnostic {
                package_id: String::from("fixture 1.0.0"),
                target_name: String::from("fixture"),
                diagnostic: serde_json::from_value(serde_json::json!({
                    "message": "unused fixture",
                    "code": null,
                    "level": "warning",
                    "spans": [],
                    "children": [],
                    "rendered": null
                }))
                .expect("rustc warning fixture decodes"),
            }],
            text_lines: Vec::new(),
            stderr: Vec::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        },
    };
    let diagnostics = super::build_diagnostics(&error);

    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Warning);
    assert_eq!(diagnostics[1].code, "cargo-upwell/build-failed");
    assert_eq!(diagnostics[1].severity, DiagnosticSeverity::Error);
}
