use std::process::Command;

use overseerd::tooling::{
    ProbeEnvelope, ProbeOutcome, TOOLING_PROBE_ARGUMENT, TOOLING_PROBE_BINARY_NAME_ENV,
    TOOLING_PROBE_MANIFEST_PATH_ENV, TOOLING_PROBE_OUTPUT_ENV, TOOLING_PROBE_PACKAGE_NAME_ENV,
    TOOLING_PROBE_PACKAGE_VERSION_ENV,
};
use overseerd_test_utils::{TempFixture, run_command};

#[test]
fn process_probe_suppresses_secret_panic_payload_everywhere() {
    let fixture = TempFixture::new("overseerd-probe-process-");
    let response = fixture.child("response.json");
    let manifest = workspace_manifest();
    let output = run_command(
        "probe fixture process",
        Command::new(tooling_probe_binary())
            .arg(TOOLING_PROBE_ARGUMENT)
            .env(TOOLING_PROBE_OUTPUT_ENV, &response)
            .env(TOOLING_PROBE_PACKAGE_NAME_ENV, env!("CARGO_PKG_NAME"))
            .env(TOOLING_PROBE_PACKAGE_VERSION_ENV, env!("CARGO_PKG_VERSION"))
            .env(TOOLING_PROBE_MANIFEST_PATH_ENV, &manifest)
            .env(TOOLING_PROBE_BINARY_NAME_ENV, "selected-thin-probe-binary"),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let json = std::fs::read_to_string(&response).expect("probe response is published");
    let envelope = ProbeEnvelope::from_json(json.trim_end()).expect("probe response validates");
    let ProbeOutcome::Failure { failure } = &envelope.outcome else {
        panic!("panicking process unexpectedly emitted success");
    };
    let diagnostic = &failure.diagnostics[0];

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout, "application stdout remains independent\n");
    assert!(!stdout.contains("{\"schema\""));
    assert!(stderr.contains("panic payload suppressed"));
    assert_eq!(diagnostic.code, "overseerd/tooling-panic");
    assert_eq!(
        diagnostic.message,
        "The tooling probe panicked while preparing the application."
    );
    assert_eq!(
        envelope
            .identity
            .binary
            .as_ref()
            .map(|binary| binary.name.as_str()),
        Some("selected-thin-probe-binary")
    );

    for output in [stdout.as_ref(), stderr.as_ref(), json.as_str()] {
        assert!(!output.contains("probe-process-secret"));
        assert!(!output.contains("api-token"));
    }
}

#[test]
fn process_probe_rejects_empty_and_invalid_invoker_identity() {
    let manifest = workspace_manifest();

    for (binary, manifest) in [
        (" ", manifest.as_os_str()),
        (
            "selected-thin-probe-binary",
            std::ffi::OsStr::new("relative/Cargo.toml"),
        ),
    ] {
        let fixture = TempFixture::new("overseerd-probe-process-");
        let response = fixture.child("response.json");
        let output = run_command(
            "probe fixture process",
            Command::new(tooling_probe_binary())
                .arg(TOOLING_PROBE_ARGUMENT)
                .env(TOOLING_PROBE_OUTPUT_ENV, &response)
                .env(TOOLING_PROBE_PACKAGE_NAME_ENV, env!("CARGO_PKG_NAME"))
                .env(TOOLING_PROBE_PACKAGE_VERSION_ENV, env!("CARGO_PKG_VERSION"))
                .env(TOOLING_PROBE_MANIFEST_PATH_ENV, manifest)
                .env(TOOLING_PROBE_BINARY_NAME_ENV, binary),
        );
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert_eq!(output.status.code(), Some(2));
        assert!(!response.exists());
        assert!(!stderr.contains("probe-process-secret"));
    }
}

fn tooling_probe_binary() -> std::path::PathBuf {
    std::env::var_os("NEXTEST_BIN_EXE_tooling_probe_fixture")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_BIN_EXE_tooling_probe_fixture")))
}

fn workspace_manifest() -> std::path::PathBuf {
    std::env::current_dir()
        .expect("test working directory is available")
        .join("Cargo.toml")
}
