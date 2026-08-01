mod common;

use cargo_overseerd::{CancellationToken, DiscoveryRequest, FeatureSelection, run_probe};
use common::{cargo_build_lock, workspace_root};
use overseerd_tooling_schema::ProbeOutcome;

#[test]
fn builds_and_consumes_a_structured_failure_probe() {
    let workspace = workspace_root();
    let request = DiscoveryRequest {
        manifest_path: Some(workspace.join("Cargo.toml")),
        current_dir: Some(workspace.clone()),
        package: Some(String::from("overseerd")),
        binary: Some(String::from("tooling_probe_fixture")),
        features: FeatureSelection {
            features: vec![String::from("cli"), String::from("tooling")],
            ..FeatureSelection::default()
        },
        ..DiscoveryRequest::default()
    };
    let _lock = cargo_build_lock();

    let result = run_probe(&request, &CancellationToken::default())
        .expect("failure envelope is a completed probe result");
    let ProbeOutcome::Failure { failure } = &result.probe.envelope.outcome else {
        panic!("panicking fixture unexpectedly succeeded");
    };

    assert_eq!(result.target.package_name, "overseerd");
    assert_eq!(result.target.binary_name, "tooling_probe_fixture");
    assert_eq!(result.probe.evidence.status.code, Some(1));
    assert_eq!(failure.diagnostics[0].code, "overseerd/tooling-panic");
    assert!(
        result
            .build
            .target_directory
            .starts_with(workspace.join("target/overseerd"))
    );

    for bytes in [
        result.probe.evidence.stdout.as_slice(),
        result.probe.evidence.stderr.as_slice(),
    ] {
        assert!(!String::from_utf8_lossy(bytes).contains("probe-process-secret"));
    }
}

#[test]
fn discovers_and_probes_the_homeledger_application() {
    let workspace = workspace_root();
    let request = DiscoveryRequest {
        manifest_path: Some(workspace.join("examples/daemon/Cargo.toml")),
        current_dir: Some(workspace.clone()),
        package: Some(String::from("overseerd-example-daemon")),
        binary: Some(String::from("overseerd-example-daemon")),
        ..DiscoveryRequest::default()
    };
    let _lock = cargo_build_lock();

    let result = run_probe(&request, &CancellationToken::default())
        .expect("Homeledger target builds and probes");
    let ProbeOutcome::Success { document } = &result.probe.envelope.outcome else {
        panic!("Homeledger probe unexpectedly failed");
    };

    assert_eq!(document.identity.application, "homeledger");
    assert_eq!(result.target.package_name, "overseerd-example-daemon");
    assert_eq!(result.target.binary_name, "overseerd-example-daemon");
    assert!(result.probe.evidence.status.success);
    assert!(result.probe.evidence.stdout.is_empty());
}
