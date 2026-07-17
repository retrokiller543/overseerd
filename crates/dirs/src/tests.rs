use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{DirectoriesManager, State};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

fn temp_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "overseerd-dirs-{tag}-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg(unix)]
#[test]
fn private_directories_are_created_with_restrictive_modes() {
    use std::os::unix::fs::PermissionsExt;

    let root = temp_path("mode");
    let state = DirectoriesManager::from_path(root.clone()).dir::<State>();

    state.ensure().expect("secure state directory");

    assert_eq!(
        std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(state.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn symlinked_application_root_is_rejected() {
    use std::os::unix::fs::symlink;

    let target = temp_path("target");
    let link = temp_path("link");
    std::fs::create_dir(&target).expect("create target");
    symlink(&target, &link).expect("create symlink");

    let state = DirectoriesManager::from_path(link.clone()).dir::<State>();
    let error = state.ensure().expect_err("symlink must be rejected");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);

    let _ = std::fs::remove_file(link);
    let _ = std::fs::remove_dir_all(target);
}

#[cfg(unix)]
#[test]
fn group_writable_existing_root_is_rejected_before_permissions_change() {
    use std::os::unix::fs::PermissionsExt;

    let root = temp_path("group-writable-target");
    std::fs::create_dir(&root).expect("create root");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o770))
        .expect("set unsafe mode");
    let state = DirectoriesManager::from_path(root.clone()).dir::<State>();

    let error = state.ensure().expect_err("unsafe target must be rejected");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(
        std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o770,
        "validation must not silently chmod an unsafe pre-existing target"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn group_writable_ancestor_is_rejected() {
    use std::os::unix::fs::PermissionsExt;

    let ancestor = temp_path("group-writable-ancestor");
    std::fs::create_dir(&ancestor).expect("create ancestor");
    std::fs::set_permissions(&ancestor, std::fs::Permissions::from_mode(0o770))
        .expect("set unsafe mode");
    let state = DirectoriesManager::from_path(ancestor.join("app")).dir::<State>();

    let error = state
        .ensure()
        .expect_err("unsafe ancestor must be rejected");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    let _ = std::fs::remove_dir_all(ancestor);
}

#[cfg(windows)]
#[test]
fn private_directories_receive_and_retain_a_private_windows_acl() {
    let root = temp_path("windows-private-acl");
    let state = DirectoriesManager::from_path(root.clone()).dir::<State>();

    state.ensure().expect("secure Windows state directory");
    state.ensure().expect("validate existing private ACL");

    assert!(root.is_dir());
    assert!(state.is_dir());
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(windows)]
#[test]
fn existing_directory_with_world_access_is_rejected() {
    let root = temp_path("windows-world-access");
    std::fs::create_dir(&root).expect("create root");
    super::windows::apply_world_access_for_test(&root).expect("set unsafe ACL");
    let state = DirectoriesManager::from_path(root.clone()).dir::<State>();

    let error = state.ensure().expect_err("unsafe ACL must be rejected");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(windows)]
#[test]
fn parent_segments_are_rejected_without_querying_an_incomplete_drive_prefix() {
    let relative = PathBuf::from("private").join("..").join("escape");
    let state = DirectoriesManager::from_path(relative).dir::<State>();

    let error = state
        .ensure()
        .expect_err("parent traversal must be rejected");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}
