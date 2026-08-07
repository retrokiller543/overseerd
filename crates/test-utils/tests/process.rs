#![cfg(unix)]

use std::process::Command;
use std::time::Duration;

use upwell_test_utils::{run_command, run_command_with_timeout};

#[test]
fn command_closes_stdin_and_drains_large_output() {
    let output = run_command(
        "large output child",
        Command::new("sh").args(["-c", "read value || true; yes x | head -c 1048576"]),
    );

    assert!(output.status.success());
    assert_eq!(output.stdout.len(), 1_048_576);
}

#[test]
fn command_terminates_descendants_that_retain_output_pipes() {
    let result = std::panic::catch_unwind(|| {
        run_command_with_timeout(
            "descendant child",
            Command::new("sh").args(["-c", "sleep 30 & wait"]),
            Duration::from_millis(100),
        )
    });

    assert!(result.is_err());
}

#[test]
fn command_reaps_descendants_after_the_direct_child_exits() {
    let output = run_command_with_timeout(
        "exited parent child",
        Command::new("sh").args(["-c", "sleep 30 & exit 0"]),
        Duration::from_secs(1),
    );

    assert!(output.status.success());
}
