use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use command_group::{CommandGroup as _, GroupChild};

/// Default upper bound for external test processes.
pub const DEFAULT_PROCESS_TIMEOUT: Duration = Duration::from_secs(300);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Runs a named command with captured output and the default timeout.
pub fn run_command(name: &str, command: &mut Command) -> Output {
    run_command_with_timeout(name, command, DEFAULT_PROCESS_TIMEOUT)
}

/// Runs a named command with captured output and a finite timeout.
pub fn run_command_with_timeout(name: &str, command: &mut Command, timeout: Duration) -> Output {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .group_spawn()
        .unwrap_or_else(|error| panic!("spawn {name}: {error}"));
    let stdout = read_in_background(child.inner().stdout.take().expect("capture child stdout"));
    let stderr = read_in_background(child.inner().stderr.take().expect("capture child stderr"));
    let deadline = Instant::now() + timeout;

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                terminate_group(name, &mut child, "after direct child exit");

                break status;
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            Ok(None) => abort_group(name, &mut child, &format!("timed out after {timeout:?}")),
            Err(error) => abort_group(name, &mut child, &format!("poll failed: {error}")),
        }
    };

    Output {
        status,
        stdout: stdout.join().expect("join child stdout reader"),
        stderr: stderr.join().expect("join child stderr reader"),
    }
}

fn read_in_background(mut stream: impl Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut output = Vec::new();

        stream.read_to_end(&mut output).expect("read child output");

        output
    })
}

fn terminate_group(name: &str, child: &mut GroupChild, reason: &str) {
    let kill_error = child.kill().err().filter(|error| {
        !matches!(
            error.kind(),
            std::io::ErrorKind::InvalidInput | std::io::ErrorKind::NotFound
        ) && !is_missing_process(error)
    });
    let wait_error = child.wait().err();

    assert!(
        kill_error.is_none() && wait_error.is_none(),
        "terminate {name} {reason}; kill={kill_error:?}; wait={wait_error:?}"
    );
}

#[cfg(unix)]
fn is_missing_process(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(libc::ESRCH)
}

#[cfg(not(unix))]
fn is_missing_process(_error: &std::io::Error) -> bool {
    false
}

fn abort_group(name: &str, child: &mut GroupChild, reason: &str) -> ! {
    terminate_group(name, child, reason);

    panic!("{name} {reason}")
}
