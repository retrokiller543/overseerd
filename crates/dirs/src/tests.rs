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
