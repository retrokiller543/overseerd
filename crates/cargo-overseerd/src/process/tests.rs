use std::process::Command;

use super::{CancellationToken, execute};

#[test]
fn pre_cancelled_work_never_launches_the_command() {
    let cancellation = CancellationToken::default();
    let mut command = Command::new("this-command-must-not-exist");

    cancellation.cancel();

    let output = execute(&mut command, &cancellation, 1024, 1024)
        .expect("pre-cancelled execution does not spawn");

    assert!(output.cancelled);
    assert_eq!(output.status.code, None);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn completion_drains_all_buffered_output() {
    let cancellation = CancellationToken::default();
    let expected = "x".repeat(256 * 1024);
    let mut command = Command::new("sh");

    command.arg("-c").arg("printf %s \"$OVERSEERD_CAPTURE\"");
    command.env("OVERSEERD_CAPTURE", &expected);

    let output = execute(&mut command, &cancellation, expected.len(), 1024)
        .expect("completed output is captured");

    assert!(output.status.success);
    assert!(!output.stdout_truncated);
    assert_eq!(output.stdout, expected.as_bytes());
}
