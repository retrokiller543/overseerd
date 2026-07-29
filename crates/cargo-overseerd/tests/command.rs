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
fn commands_run_from_the_selected_crate_with_workspace_relative_defaults() {
    let example = workspace_root().join("examples/daemon");
    let binary = env!("CARGO_BIN_EXE_cargo-overseerd");

    for command in ["check", "doctor"] {
        let output = Command::new(binary)
            .arg(command)
            .current_dir(&example)
            .output()
            .expect("cargo-overseerd command launches");

        assert!(
            output.status.success(),
            "{command} failed: {}",
            String::from_utf8_lossy(&output.stdout)
        );

        let stdout = String::from_utf8(output.stdout).expect("command output is UTF-8");

        assert!(stdout.contains("Application homeledger (homeledger/rpc)"));
        assert!(stdout.contains(&format!("cargo overseerd {command} passed")));
        assert!(output.stderr.is_empty());
    }
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

    assert_eq!(report["schema"], env!("CARGO_PKG_VERSION"));
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

#[test]
fn inspect_json_and_document_export_are_byte_identical() {
    let workspace = workspace_root();
    let inspect = run_homeledger(&workspace, ["inspect", "--format", "json"]);
    let export = run_homeledger(&workspace, ["export", "--format", "document"]);

    assert!(inspect.status.success());
    assert!(export.status.success());
    assert_eq!(inspect.stdout, export.stdout);
    assert!(inspect.stderr.is_empty());
    assert!(export.stderr.is_empty());

    let document: overseerd_tooling_schema::ToolingDocument =
        serde_json::from_slice(&export.stdout).expect("document export parses");

    document.validate().expect("document export validates");
    assert_eq!(
        document.schema,
        semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("package version is semantic")
    );
    assert_eq!(document.identity.application, "homeledger");
    assert!(document.resources.iter().any(|resource| {
        resource.kind == overseerd_tooling_schema::ResourceKind::ConfigBinding
            && resource.labels.get("redacted").map(String::as_str) == Some("true")
            && resource.labels.get("value-exported").map(String::as_str) == Some("false")
    }));
}

#[test]
fn envelope_export_preserves_structured_probe_failure_without_stdout_noise() {
    let workspace = workspace_root();
    let binary = env!("CARGO_BIN_EXE_cargo-overseerd");
    let output = Command::new(binary)
        .arg("export")
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .arg("--package")
        .arg("overseerd")
        .arg("--bin")
        .arg("tooling_probe_fixture")
        .arg("--features")
        .arg("cli,tooling")
        .arg("--format")
        .arg("envelope")
        .current_dir(&workspace)
        .output()
        .expect("cargo-overseerd export launches");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());

    let envelope = overseerd_tooling_schema::ProbeEnvelope::from_json(
        std::str::from_utf8(&output.stdout).expect("envelope export is UTF-8"),
    )
    .expect("failure envelope validates");

    assert!(!envelope.is_success());
}

#[test]
fn document_export_failure_keeps_stdout_empty() {
    let workspace = workspace_root();
    let binary = env!("CARGO_BIN_EXE_cargo-overseerd");
    let output = Command::new(binary)
        .arg("export")
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .arg("--package")
        .arg("overseerd")
        .arg("--bin")
        .arg("tooling_probe_fixture")
        .arg("--features")
        .arg("cli,tooling")
        .current_dir(&workspace)
        .output()
        .expect("cargo-overseerd export launches");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}

#[test]
fn inspect_filters_are_rejected_for_canonical_json() {
    let workspace = workspace_root();
    let output = run_homeledger(
        &workspace,
        ["inspect", "--format", "json", "--kind", "component"],
    );

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}

#[test]
fn document_export_file_matches_stdout_bytes() {
    let workspace = workspace_root();
    let stdout = run_homeledger(&workspace, ["export", "--format", "document"]);
    let output_path = std::env::temp_dir().join(format!(
        "cargo-overseerd-export-{}-{}.json",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let binary = env!("CARGO_BIN_EXE_cargo-overseerd");
    let file = Command::new(binary)
        .arg("export")
        .arg("--format")
        .arg("document")
        .arg("--output")
        .arg(&output_path)
        .arg("--manifest-path")
        .arg(workspace.join("examples/daemon/Cargo.toml"))
        .arg("--package")
        .arg("overseerd-example-daemon")
        .arg("--bin")
        .arg("overseerd-example-daemon")
        .current_dir(&workspace)
        .output()
        .expect("file export launches");
    let file_bytes = std::fs::read(&output_path).expect("file export is readable");

    std::fs::remove_file(&output_path).expect("file export is removable");

    assert!(stdout.status.success());
    assert!(file.status.success());
    assert!(file.stdout.is_empty());
    assert!(file.stderr.is_empty());
    assert_eq!(file_bytes, stdout.stdout);
}

#[cfg(unix)]
#[test]
fn replacing_export_preserves_existing_file_permissions() {
    use std::os::unix::fs::PermissionsExt as _;

    let workspace = workspace_root();
    let output_path = std::env::temp_dir().join(format!(
        "cargo-overseerd-private-export-{}.json",
        std::process::id()
    ));

    std::fs::write(&output_path, b"previous").expect("previous export writes");
    std::fs::set_permissions(&output_path, std::fs::Permissions::from_mode(0o600))
        .expect("private permissions apply");

    let binary = env!("CARGO_BIN_EXE_cargo-overseerd");
    let output = Command::new(binary)
        .arg("export")
        .arg("--output")
        .arg(&output_path)
        .arg("--manifest-path")
        .arg(workspace.join("examples/daemon/Cargo.toml"))
        .arg("--package")
        .arg("overseerd-example-daemon")
        .arg("--bin")
        .arg("overseerd-example-daemon")
        .current_dir(&workspace)
        .output()
        .expect("private file export launches");
    let mode = std::fs::metadata(&output_path)
        .expect("private export metadata reads")
        .permissions()
        .mode()
        & 0o777;

    std::fs::remove_file(&output_path).expect("private export is removable");

    assert!(output.status.success());
    assert_eq!(mode, 0o600);
}

fn run_homeledger<const N: usize>(workspace: &Path, arguments: [&str; N]) -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_cargo-overseerd");

    Command::new(binary)
        .args(arguments)
        .arg("--manifest-path")
        .arg(workspace.join("examples/daemon/Cargo.toml"))
        .arg("--package")
        .arg("overseerd-example-daemon")
        .arg("--bin")
        .arg("overseerd-example-daemon")
        .current_dir(workspace)
        .output()
        .expect("cargo-overseerd Homeledger command launches")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("cargo-overseerd belongs to the repository workspace")
        .to_path_buf()
}
