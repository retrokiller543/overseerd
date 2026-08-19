use std::fs;
use std::path::{Component, Path, PathBuf};

use tempfile::{Builder, TempDir};

/// Owns a uniquely named temporary filesystem fixture.
pub struct TempFixture {
    root: TempDir,
}

impl TempFixture {
    /// Creates an automatically removed fixture with a descriptive prefix.
    pub fn new(prefix: &str) -> Self {
        let root = Builder::new()
            .prefix(prefix)
            .tempdir()
            .expect("create temporary test fixture");

        Self { root }
    }

    /// Returns the fixture root.
    pub fn path(&self) -> &Path {
        self.root.path()
    }

    /// Returns a path below the fixture root.
    pub fn child(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = path.as_ref();

        assert!(
            path.components()
                .all(|component| matches!(component, Component::Normal(_) | Component::CurDir)),
            "temporary fixture children must be relative and cannot contain parent components"
        );

        self.path().join(path)
    }

    /// Writes a file below the fixture root, creating parent directories.
    pub fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = self.child(path);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create temporary fixture parent");
        }

        fs::write(&path, contents).expect("write temporary fixture file");

        path
    }
}
