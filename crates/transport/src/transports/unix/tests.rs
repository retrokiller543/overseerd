use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use super::UnixTransport;
use overseerd_test_utils::{TempFixture, run_command_with_timeout};

const RELATIVE_SOCKET_HELPER: &str =
    "transports::unix::tests::relative_socket_path_without_a_parent_helper";
const RELATIVE_SOCKET_HELPER_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test]
async fn socket_and_parent_are_private() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = TempFixture::new("overseerd-unix-mode-");
    let parent = fixture.child("socket");
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

    let fixture = TempFixture::new("overseerd-unix-symlink-");
    let link = fixture.child("link");
    let target = fixture.child("target");
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

    let fixture = TempFixture::new("overseerd-unix-intermediate-");
    let base = fixture.child("base");
    let target = fixture.child("target");
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
    let fixture = TempFixture::new("overseerd-unix-relative-");
    let mut command = Command::new(std::env::current_exe().expect("locate test executable"));

    command
        .arg("--ignored")
        .arg("--exact")
        .arg(RELATIVE_SOCKET_HELPER)
        .arg("--nocapture")
        .current_dir(fixture.path());

    let output = run_command_with_timeout(
        "relative socket child test",
        &mut command,
        RELATIVE_SOCKET_HELPER_TIMEOUT,
    );

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

    let fixture = TempFixture::new("overseerd-unix-group-writable-parent-");
    let parent = fixture.child("parent");
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

    let fixture = TempFixture::new("overseerd-unix-group-writable-ancestor-");
    let ancestor = fixture.child("ancestor");
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
