#[cfg(windows)]
use std::path::PathBuf;

use super::{DirectoriesManager, State};
use tempfile::TempDir;

fn temp_dir(tag: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(&format!("upwell-dirs-{tag}-"))
        .tempdir()
        .expect("create temp directory")
}

#[cfg(unix)]
#[test]
fn private_directories_are_created_with_restrictive_modes() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = temp_dir("mode");
    let root = fixture.path().join("app");
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
}

#[cfg(unix)]
#[test]
fn symlinked_application_root_is_rejected() {
    use std::os::unix::fs::symlink;

    let fixture = temp_dir("symlink");
    let target = fixture.path().join("target");
    let link = fixture.path().join("link");
    std::fs::create_dir(&target).expect("create target");
    symlink(&target, &link).expect("create symlink");

    let state = DirectoriesManager::from_path(link.clone()).dir::<State>();
    let error = state.ensure().expect_err("symlink must be rejected");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
}

#[cfg(unix)]
#[test]
fn group_writable_existing_root_is_rejected_before_permissions_change() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = temp_dir("group-writable-target");
    let root = fixture.path().join("app");
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
}

#[cfg(unix)]
#[test]
fn group_writable_ancestor_is_rejected() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = temp_dir("group-writable-ancestor");
    let ancestor = fixture.path().join("ancestor");
    std::fs::create_dir(&ancestor).expect("create ancestor");
    std::fs::set_permissions(&ancestor, std::fs::Permissions::from_mode(0o770))
        .expect("set unsafe mode");
    let state = DirectoriesManager::from_path(ancestor.join("app")).dir::<State>();

    let error = state
        .ensure()
        .expect_err("unsafe ancestor must be rejected");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
}

#[cfg(windows)]
#[test]
fn private_directories_receive_and_retain_a_private_windows_acl() {
    let fixture = temp_dir("windows-private-acl");
    let root = fixture.path().join("app");
    let renamed = root.with_extension("renamed");
    let state = DirectoriesManager::from_path(root.clone()).dir::<State>();

    state.ensure().expect("secure Windows state directory");
    state.ensure().expect("validate existing private ACL");

    assert!(root.is_dir());
    assert!(state.is_dir());
    assert!(
        std::fs::rename(&root, &renamed).is_err(),
        "ensured directory remains protected from ancestor replacement"
    );

    drop(state);
    std::fs::rename(&root, &renamed).expect("protection releases with the Dir handle");
}

#[cfg(windows)]
#[test]
fn existing_directory_with_world_access_is_rejected() {
    let fixture = temp_dir("windows-world-access");
    let root = fixture.path().join("app");
    std::fs::create_dir(&root).expect("create root");
    super::windows::apply_world_access_for_test(&root).expect("set unsafe ACL");
    let state = DirectoriesManager::from_path(root.clone()).dir::<State>();

    let error = state.ensure().expect_err("unsafe ACL must be rejected");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    drop(state);
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
