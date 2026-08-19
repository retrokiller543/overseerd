use std::fs::{File, OpenOptions};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;

pub(super) fn create_private(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();

    options.write(true).create_new(true);

    #[cfg(unix)]
    options.mode(0o600);

    options.open(path)
}

/// Atomically adds the final name only when it is absent.
///
/// Both paths are siblings and the source is a regular file, so a hard link is an atomic
/// no-clobber publication of the already synchronized inode on every supported platform.
pub(super) fn publish_no_replace(temporary: &Path, final_path: &Path) -> std::io::Result<()> {
    std::fs::hard_link(temporary, final_path)
}

#[cfg(unix)]
pub(super) fn sync_parent(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
pub(super) fn sync_parent(_path: &Path) -> std::io::Result<()> {
    Ok(())
}
