use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::UnixTransport;
use tempfile::TempDir;

const RELATIVE_SOCKET_HELPER: &str =
    "transports::unix::tests::relative_socket_path_without_a_parent_helper";
const RELATIVE_SOCKET_HELPER_TIMEOUT: Duration = Duration::from_secs(5);
const RELATIVE_SOCKET_HELPER_POLL_INTERVAL: Duration = Duration::from_millis(10);

fn temp_dir(tag: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(&format!("overseerd-unix-{tag}-"))
        .tempdir()
        .expect("create temp directory")
}

fn child_output(child: &mut Child, status: ExitStatus) -> Output {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    child
        .stdout
        .take()
        .expect("capture relative socket child stdout")
        .read_to_end(&mut stdout)
        .expect("read relative socket child stdout");
    child
        .stderr
        .take()
        .expect("capture relative socket child stderr")
        .read_to_end(&mut stderr)
        .expect("read relative socket child stderr");

    Output {
        status,
        stdout,
        stderr,
    }
}

fn abort_child(mut child: Child, reason: &str) -> ! {
    let kill_error = child.kill().err();
    let wait_result = child.wait();
    let (status, stdout, stderr) = match wait_result {
        Ok(status) => {
            let output = child_output(&mut child, status);

            (Some(output.status), output.stdout, output.stderr)
        }
        Err(error) => (None, Vec::new(), error.to_string().into_bytes()),
    };

    panic!(
        "{reason}\nkill error: {kill_error:?}\nstatus: {status:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr),
    );
}

fn wait_for_child(mut child: Child) -> Output {
    let deadline = Instant::now() + RELATIVE_SOCKET_HELPER_TIMEOUT;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return child_output(&mut child, status),
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(RELATIVE_SOCKET_HELPER_POLL_INTERVAL);
            }
            Ok(None) => abort_child(child, "relative socket child timed out"),
            Err(error) => abort_child(
                child,
                &format!("failed to poll relative socket child: {error}"),
            ),
        }
    }
}

#[tokio::test]
async fn socket_and_parent_are_private() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = temp_dir("mode");
    let parent = fixture.path().join("socket");
    let path = parent.join("daemon.sock");
    let transport = UnixTransport::bind(path.clone()).expect("bind Unix socket");

    assert_eq!(
        std::fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    drop(transport);
    assert!(!path.exists(), "dropping the transport removes its socket");
}

#[test]
fn symlinked_socket_parent_is_rejected() {
    use std::os::unix::fs::symlink;

    let fixture = temp_dir("symlink");
    let link = fixture.path().join("link");
    let target = fixture.path().join("target");
    let path = link.join("daemon.sock");
    std::fs::create_dir(&target).expect("create target");
    symlink(&target, &link).expect("create symlink");

    let error = match UnixTransport::bind(path) {
        Ok(_) => panic!("symlink must be rejected"),
        Err(error) => error,
    };

    assert!(
        matches!(error, crate::Error::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied)
    );
}

#[test]
fn intermediate_symlink_in_socket_path_is_rejected() {
    use std::os::unix::fs::symlink;

    let fixture = temp_dir("intermediate");
    let base = fixture.path().join("base");
    let target = fixture.path().join("target");
    let link = base.join("link");
    std::fs::create_dir_all(&base).expect("create base");
    std::fs::create_dir(&target).expect("create target");
    symlink(&target, &link).expect("create intermediate symlink");
    let socket = link.join("nested").join("daemon.sock");

    let error = match UnixTransport::bind(socket) {
        Ok(_) => panic!("intermediate symlink must be rejected"),
        Err(error) => error,
    };

    assert!(
        matches!(error, crate::Error::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied)
    );
}

fn assert_relative_socket_binds() {
    use std::os::unix::fs::PermissionsExt;

    let path = PathBuf::from("daemon.sock");
    let transport = UnixTransport::bind(path.clone()).expect("bind relative socket");

    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    drop(transport);
    assert!(!path.exists(), "dropping the transport removes its socket");
}

#[tokio::test]
#[ignore = "run in an isolated working directory by the parent test"]
async fn relative_socket_path_without_a_parent_helper() {
    assert_relative_socket_binds();
}

#[test]
fn relative_socket_path_without_a_parent_still_binds() {
    let fixture = temp_dir("relative");
    let child = Command::new(std::env::current_exe().expect("locate test executable"))
        .arg("--ignored")
        .arg("--exact")
        .arg(RELATIVE_SOCKET_HELPER)
        .arg("--nocapture")
        .current_dir(fixture.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run relative socket child test");
    let output = wait_for_child(child);

    assert!(
        output.status.success(),
        "relative socket child failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn group_writable_socket_parent_is_rejected_before_permissions_change() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = temp_dir("group-writable-parent");
    let parent = fixture.path().join("parent");
    let path = parent.join("daemon.sock");
    std::fs::create_dir(&parent).expect("create parent");
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o770))
        .expect("set unsafe mode");

    let error = match UnixTransport::bind(path) {
        Ok(_) => panic!("unsafe parent must be rejected"),
        Err(error) => error,
    };

    assert!(
        matches!(error, crate::Error::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied)
    );
    assert_eq!(
        std::fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
        0o770
    );
}

#[test]
fn group_writable_socket_ancestor_is_rejected() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = temp_dir("group-writable-ancestor");
    let ancestor = fixture.path().join("ancestor");
    let path = ancestor.join("parent").join("daemon.sock");
    std::fs::create_dir(&ancestor).expect("create ancestor");
    std::fs::set_permissions(&ancestor, std::fs::Permissions::from_mode(0o770))
        .expect("set unsafe mode");

    let error = match UnixTransport::bind(path) {
        Ok(_) => panic!("unsafe ancestor must be rejected"),
        Err(error) => error,
    };

    assert!(
        matches!(error, crate::Error::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied)
    );
}
