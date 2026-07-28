use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn cargo_subcommand_reports_the_live_homeledger_application() {
    let workspace = workspace_root();
    let binary = env!("CARGO_BIN_EXE_cargo-overseerd");
    let output = Command::new(binary)
        .arg("overseerd")
        .arg("check")
        .arg("--manifest-path")
        .arg(workspace.join("examples/daemon/Cargo.toml"))
        .arg("--package")
        .arg("overseerd-example-daemon")
        .arg("--bin")
        .arg("overseerd-example-daemon")
        .current_dir(&workspace)
        .output()
        .expect("cargo-overseerd command launches");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("command output is UTF-8");

    assert!(stdout.contains("Application homeledger (homeledger/rpc)"));
    assert!(stdout.contains("Framework Overseerd 0.20.0"));
    assert!(stdout.contains("cargo overseerd check passed"));
    assert!(output.stderr.is_empty());
}

#[test]
fn json_validation_failure_uses_the_stable_exit_code_and_diagnostic() {
    let workspace = workspace_root();
    let binary = env!("CARGO_BIN_EXE_cargo-overseerd");
    let output = Command::new(binary)
        .arg("check")
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .arg("--package")
        .arg("overseerd")
        .arg("--bin")
        .arg("tooling_probe_fixture")
        .arg("--features")
        .arg("cli,tooling")
        .arg("--format")
        .arg("json")
        .current_dir(&workspace)
        .output()
        .expect("cargo-overseerd command launches");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("JSON report parses");

    assert_eq!(report["schema"]["major"], 1);
    assert_eq!(report["outcome"], "validation-failure");
    assert_eq!(report["exit_code"], 1);
    assert_eq!(report["diagnostics"][0]["code"], "overseerd/tooling-panic");
}

#[test]
fn invalid_command_uses_the_stable_misuse_exit_code() {
    let binary = env!("CARGO_BIN_EXE_cargo-overseerd");
    let output = Command::new(binary)
        .arg("unknown-command")
        .output()
        .expect("cargo-overseerd command launches");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("cargo-overseerd belongs to the repository workspace")
        .to_path_buf()
}
