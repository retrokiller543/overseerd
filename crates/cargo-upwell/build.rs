use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    watch_git_state();
    watch_tracked_files();

    emit("CARGO_UPWELL_BUILD_TARGET", std::env::var("TARGET").ok());
    emit("CARGO_UPWELL_BUILD_PROFILE", std::env::var("PROFILE").ok());
    emit(
        "CARGO_UPWELL_RUSTC_VERSION",
        command_output(
            std::env::var("RUSTC").unwrap_or_else(|_| String::from("rustc")),
            ["--version"],
        ),
    );
    emit(
        "CARGO_UPWELL_GIT_COMMIT",
        std::env::var("GITHUB_SHA")
            .ok()
            .or_else(|| command_output("git", ["rev-parse", "HEAD"])),
    );
    emit(
        "CARGO_UPWELL_GIT_DIRTY",
        git_dirty().map(|dirty| dirty.to_string()),
    );
}

fn emit(name: &str, value: Option<String>) {
    println!(
        "cargo:rustc-env={name}={}",
        value.as_deref().unwrap_or("unknown")
    );
}

fn command_output<const N: usize>(
    program: impl AsRef<std::ffi::OsStr>,
    arguments: [&str; N],
) -> Option<String> {
    let output = Command::new(program).args(arguments).output().ok()?;

    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn git_dirty() -> Option<bool> {
    let status = Command::new("git")
        .args(["diff", "--quiet", "--ignore-submodules", "HEAD", "--"])
        .status()
        .ok()?;

    match status.code() {
        Some(0) => Some(false),
        Some(1) => Some(true),
        _ => None,
    }
}

fn watch_git_state() {
    for path in ["HEAD", "index"] {
        if let Some(path) = command_output("git", ["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = command_output("git", ["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = command_output("git", ["rev-parse", "--git-path", &reference])
    {
        println!("cargo:rerun-if-changed={path}");
    }
}

fn watch_tracked_files() {
    let Some(files) = command_output("git", ["ls-files"]) else {
        return;
    };

    for file in files.lines() {
        println!("cargo:rerun-if-changed={file}");
    }
}
