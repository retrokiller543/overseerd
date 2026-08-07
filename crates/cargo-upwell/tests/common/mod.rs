use std::fs::{File, OpenOptions};
use std::path::PathBuf;

use fs2::FileExt as _;

pub fn cargo_build_lock() -> File {
    let workspace = workspace_root();
    let lock_path = workspace.join("target/cargo-upwell-tests.lock");

    std::fs::create_dir_all(lock_path.parent().expect("lock path has a parent"))
        .expect("create cargo-upwell test lock directory");

    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)
        .expect("open cargo-upwell test lock");

    lock.lock_exclusive()
        .expect("lock cargo-upwell test builds");

    lock
}

pub fn workspace_root() -> PathBuf {
    let current = std::env::current_dir().expect("test working directory is available");

    for directory in current.ancestors() {
        let manifest = directory.join("Cargo.toml");

        if manifest.is_file()
            && std::fs::read_to_string(&manifest)
                .expect("workspace manifest is readable")
                .contains("[workspace]")
        {
            return directory.to_path_buf();
        }
    }

    panic!("cargo-upwell tests run below the repository workspace")
}
