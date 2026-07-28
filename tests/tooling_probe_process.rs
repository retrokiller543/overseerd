use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use overseerd::tooling::{
    ProbeEnvelope, ProbeOutcome, TOOLING_PROBE_ARGUMENT, TOOLING_PROBE_BINARY_NAME_ENV,
    TOOLING_PROBE_MANIFEST_PATH_ENV, TOOLING_PROBE_OUTPUT_ENV, TOOLING_PROBE_PACKAGE_NAME_ENV,
    TOOLING_PROBE_PACKAGE_VERSION_ENV,
};

#[test]
fn process_probe_suppresses_secret_panic_payload_everywhere() {
    let directory = probe_directory();
    let response = directory.join("response.json");
    let output = Command::new(env!("CARGO_BIN_EXE_tooling_probe_fixture"))
        .arg(TOOLING_PROBE_ARGUMENT)
        .env(TOOLING_PROBE_OUTPUT_ENV, &response)
        .env(TOOLING_PROBE_PACKAGE_NAME_ENV, env!("CARGO_PKG_NAME"))
        .env(TOOLING_PROBE_PACKAGE_VERSION_ENV, env!("CARGO_PKG_VERSION"))
        .env(
            TOOLING_PROBE_MANIFEST_PATH_ENV,
            concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
        )
        .env(TOOLING_PROBE_BINARY_NAME_ENV, "selected-thin-probe-binary")
        .output()
        .expect("probe fixture process executes");
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

    std::fs::remove_dir_all(directory).expect("probe fixture directory is removed");
}

#[test]
fn process_probe_rejects_empty_and_invalid_invoker_identity() {
    for (binary, manifest) in [
        (" ", concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")),
        ("selected-thin-probe-binary", "relative/Cargo.toml"),
    ] {
        let directory = probe_directory();
        let response = directory.join("response.json");
        let output = Command::new(env!("CARGO_BIN_EXE_tooling_probe_fixture"))
            .arg(TOOLING_PROBE_ARGUMENT)
            .env(TOOLING_PROBE_OUTPUT_ENV, &response)
            .env(TOOLING_PROBE_PACKAGE_NAME_ENV, env!("CARGO_PKG_NAME"))
            .env(TOOLING_PROBE_PACKAGE_VERSION_ENV, env!("CARGO_PKG_VERSION"))
            .env(TOOLING_PROBE_MANIFEST_PATH_ENV, manifest)
            .env(TOOLING_PROBE_BINARY_NAME_ENV, binary)
            .output()
            .expect("probe fixture process executes");
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert_eq!(output.status.code(), Some(2));
        assert!(!response.exists());
        assert!(!stderr.contains("probe-process-secret"));

        std::fs::remove_dir_all(directory).expect("probe fixture directory is removed");
    }
}

fn probe_directory() -> std::path::PathBuf {
    static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    let ordinal = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "overseerd-probe-process-{}-{ordinal}",
        std::process::id()
    ));

    std::fs::create_dir(&path).expect("probe fixture directory is created");

    path
}
