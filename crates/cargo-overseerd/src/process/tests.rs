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
