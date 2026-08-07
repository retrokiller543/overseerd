use std::process::Command;

use super::{CancellationToken, execute};

#[cfg(unix)]
use super::execute_with_mirror_writer;
#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::sync::{Arc, Mutex};

#[cfg(unix)]
#[derive(Clone, Default)]
struct SharedWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
}

#[cfg(unix)]
struct FailingWriter;

#[cfg(unix)]
impl io::Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("presentation sink failed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(unix)]
impl io::Write for SharedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.bytes.lock().expect("writer lock").extend(buffer);

        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

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

    command
        .arg("-c")
        .arg("dd if=/dev/zero bs=262144 count=1 2>/dev/null | tr '\\0' x");

    let output = execute(&mut command, &cancellation, expected.len(), 1024)
        .expect("completed output is captured");

    assert!(output.status.success);
    assert!(!output.stdout_truncated);
    assert_eq!(output.stdout, expected.as_bytes());
}

#[cfg(unix)]
#[test]
fn stderr_mirroring_keeps_complete_capture() {
    let cancellation = CancellationToken::default();
    let mirror = SharedWriter::default();
    let mirrored = mirror.clone();
    let mut command = Command::new("sh");

    command.arg("-c").arg("printf 'building\\n' >&2");

    let output =
        execute_with_mirror_writer(&mut command, &cancellation, 1024, 1024, Box::new(mirror))
            .expect("mirrored execution completes");

    assert_eq!(output.stderr, b"building\n");
    assert_eq!(
        mirrored.bytes.lock().expect("writer lock").as_slice(),
        b"building\n"
    );
}

#[cfg(unix)]
#[test]
fn mirror_failure_never_changes_capture_or_process_success() {
    let cancellation = CancellationToken::default();
    let mut command = Command::new("sh");

    command.arg("-c").arg("printf 'building\\n' >&2");

    let output = execute_with_mirror_writer(
        &mut command,
        &cancellation,
        1024,
        1024,
        Box::new(FailingWriter),
    )
    .expect("presentation failure does not fail execution");

    assert!(output.status.success);
    assert_eq!(output.stderr, b"building\n");
}
