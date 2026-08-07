mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use upwell_test_utils::{TempFixture, run_command};

static GENERATED_PROJECTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn built_in_application_is_immediately_usable_by_all_tooling_surfaces() {
    let _serial = GENERATED_PROJECTS
        .lock()
        .expect("generated project tests are serialized");
    let _guard = common::cargo_build_lock();
    let fixture = TempFixture::new("cargo-upwell-init-application");
    let project = fixture.child("sample-app");

    let output = cargo_upwell(
        fixture.path(),
        [
            "init",
            project.to_str().expect("fixture path is UTF-8"),
            "--name",
            "sample-app",
            "--upwell-path",
            workspace_root().to_str().expect("workspace path is UTF-8"),
            "--no-vcs",
        ],
    );

    assert_success(&output, "application generation");
    assert!(project.join("src/lib.rs").is_file());
    assert!(project.join("src/main.rs").is_file());

    if !dependencies_available(&project) {
        return;
    }

    assert_success(
        &cargo(&project, ["test", "--all-features"]),
        "generated application tests",
    );
    assert_success(
        &cargo(&project, ["run", "--", "about"]),
        "generated application CLI",
    );

    for arguments in [
        vec!["check"],
        vec!["inspect"],
        vec!["graph"],
        vec!["explain", "sample-app"],
    ] {
        assert_success(
            &cargo_upwell(&project, arguments),
            "generated application tooling",
        );
    }

    assert_success(
        &cargo_upwell(&project, ["check", "--no-default-features"]),
        "CLI-disabled tooling probe",
    );
}

#[test]
fn built_in_workspace_plugin_and_protocol_templates_compile() {
    let _serial = GENERATED_PROJECTS
        .lock()
        .expect("generated project tests are serialized");
    let _guard = common::cargo_build_lock();
    let fixture = TempFixture::new("cargo-upwell-init-builtins");
    let root = workspace_root();

    for (id, name) in [
        ("upwell/application-workspace", "sample-workspace"),
        ("upwell/plugin", "sample-plugin"),
        ("upwell/protocol", "sample-protocol"),
    ] {
        let project = fixture.child(name);
        let output = cargo_upwell(
            fixture.path(),
            [
                "init",
                project.to_str().expect("fixture path is UTF-8"),
                "--name",
                name,
                "--template",
                id,
                "--upwell-path",
                root.to_str().expect("workspace path is UTF-8"),
                "--no-vcs",
            ],
        );

        assert_success(&output, id);
        if dependencies_available(&project) {
            assert_success(&cargo(&project, ["test"]), id);
        }
    }
}

#[test]
fn workspace_registration_adds_the_committed_project_once() {
    let _serial = GENERATED_PROJECTS
        .lock()
        .expect("generated project tests are serialized");
    let _guard = common::cargo_build_lock();
    let fixture = TempFixture::new("cargo-upwell-init-workspace-member");
    let project = fixture.child("sample-member");

    fixture.write(
        "Cargo.toml",
        "[workspace]\nmembers = []\nresolver = \"3\"\n",
    );

    let output = cargo_upwell(
        fixture.path(),
        [
            "init",
            project.to_str().expect("fixture path is UTF-8"),
            "--name",
            "sample-member",
            "--template",
            "upwell/plugin",
            "--upwell-path",
            workspace_root().to_str().expect("workspace path is UTF-8"),
            "--workspace",
            "--no-vcs",
        ],
    );

    assert_success(&output, "workspace project generation");
    let manifest = std::fs::read_to_string(fixture.child("Cargo.toml"))
        .expect("workspace manifest is readable");

    assert!(manifest.contains("\"sample-member\""));
    if dependencies_available(fixture.path()) {
        assert_success(
            &cargo(fixture.path(), ["test", "--workspace"]),
            "workspace member tests",
        );
    }
}

#[test]
fn failed_workspace_registration_removes_the_generated_project() {
    let _serial = GENERATED_PROJECTS
        .lock()
        .expect("generated project tests are serialized");
    let _guard = common::cargo_build_lock();
    let fixture = TempFixture::new("cargo-upwell-init-missing-workspace");
    let project = fixture.child("sample-member");

    let output = cargo_upwell(
        fixture.path(),
        [
            "init",
            project.to_str().expect("fixture path is UTF-8"),
            "--name",
            "sample-member",
            "--template",
            "upwell/plugin",
            "--upwell-path",
            workspace_root().to_str().expect("workspace path is UTF-8"),
            "--workspace",
            "--no-vcs",
        ],
    );

    assert!(!output.status.success());
    assert!(!project.exists());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("failed to add"),
        "unexpected error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn cargo_upwell<I, S>(current_dir: &Path, arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut command = Command::new(cargo_upwell_executable());

    command.current_dir(current_dir).args(arguments);
    configure_nested_cargo(&mut command);

    run_command("cargo-upwell", &mut command)
}

fn cargo<I, S>(current_dir: &Path, arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));

    command.current_dir(current_dir).args(arguments);
    configure_nested_cargo(&mut command);

    run_command("cargo", &mut command)
}

fn configure_nested_cargo(command: &mut Command) {
    command.env("CARGO_NET_OFFLINE", "true").env(
        "CARGO_TARGET_DIR",
        workspace_root().join("target/init-tests"),
    );
}

fn dependencies_available(project: &Path) -> bool {
    if std::env::var_os("UPWELL_SKIP_GENERATED_PROJECT_BUILDS").is_some() {
        return false;
    }

    cargo(project, ["generate-lockfile"]).status.success()
}

fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn workspace_root() -> PathBuf {
    common::workspace_root()
}

fn cargo_upwell_executable() -> PathBuf {
    std::env::var_os("NEXTEST_BIN_EXE_cargo-upwell")
        .or_else(|| std::env::var_os("CARGO_BIN_EXE_cargo-upwell"))
        .map(PathBuf::from)
        .expect("cargo-upwell test executable is available")
}
