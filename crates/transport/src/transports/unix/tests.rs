use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::UnixTransport;

static NEXT_SOCKET: AtomicUsize = AtomicUsize::new(0);

fn socket_path(tag: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "overseerd-unix-{tag}-{}-{}",
            std::process::id(),
            NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)
        ))
        .join("daemon.sock")
}

#[tokio::test]
async fn socket_and_parent_are_private() {
    use std::os::unix::fs::PermissionsExt;

    let path = socket_path("mode");
    let parent = path.parent().unwrap().to_path_buf();
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
    let _ = std::fs::remove_dir_all(parent);
}

#[test]
fn symlinked_socket_parent_is_rejected() {
    use std::os::unix::fs::symlink;

    let path = socket_path("symlink");
    let link = path.parent().unwrap().to_path_buf();
    let target = link.with_extension("target");
    std::fs::create_dir(&target).expect("create target");
    symlink(&target, &link).expect("create symlink");

    let error = match UnixTransport::bind(path) {
        Ok(_) => panic!("symlink must be rejected"),
        Err(error) => error,
    };

    assert!(
        matches!(error, crate::Error::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied)
    );

    let _ = std::fs::remove_file(link);
    let _ = std::fs::remove_dir_all(target);
}

#[test]
fn intermediate_symlink_in_socket_path_is_rejected() {
    use std::os::unix::fs::symlink;

    let path = socket_path("intermediate");
    let base = path.parent().unwrap().to_path_buf();
    let target = base.with_extension("target");
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

    let _ = std::fs::remove_file(link);
    let _ = std::fs::remove_dir_all(base);
    let _ = std::fs::remove_dir_all(target);
}
