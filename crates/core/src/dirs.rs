//! Unified application directories, injectable as `Dir<K>`.
//!
//! A [`DirectoriesManager`] (built from project metadata, typically in `main`)
//! resolves the platform's per-application directories — config, data, cache, state,
//! runtime, and temp — via the `directories` crate's XDG / Known-Folder logic. Each
//! is handed out as a typed [`Dir<K>`] keyed by a marker ([`Config`], [`Runtime`],
//! [`Tmp`], …), so a component injects exactly the directory it needs:
//!
//! ```ignore
//! #[component]
//! struct Store { data: Dir<Data> }
//! ```
//!
//! The same manager is consumed by [`ConfigManager`](crate::ConfigManager) (to locate
//! config files) and seeded into the daemon, so directory resolution is defined once.

use std::marker::PhantomData;
use std::ops::Deref;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::descriptors::{Component, Injectable};

/// A kind of application directory. Implemented by the marker types and used by
/// [`DirectoriesManager`] to resolve the concrete path for a [`Dir<Self>`].
///
/// `project_path` resolves against the platform project dirs (with a sensible
/// fallback baked in for kinds that some platforms lack); `rooted_path` is the
/// fallback layout used when no home directory is available.
pub trait DirKind: Send + Sync + 'static {
    /// Name to be displayed when printing the dep graph
    const NAME: &'static str;
    /// Short label — the on-disk subdirectory name and the display name.
    const LABEL: &'static str;
    /// Unique dependency-injection id (namespaced to avoid colliding with user
    /// component ids).
    const COMPONENT_ID: &'static str;

    /// Resolves this kind's path from the platform project dirs.
    fn project_path(dirs: &ProjectDirs) -> PathBuf;

    /// Resolves this kind's path under a single fallback root (used when there is no
    /// home directory).
    fn rooted_path(root: &Path) -> PathBuf {
        root.join(Self::LABEL)
    }
}

/// Generates the directory marker types and their [`DirKind`] impls.
macro_rules! dir_kinds {
    ($($(#[$meta:meta])* $name:ident => $label:literal, $project:expr;)*) => {
        $(
            $(#[$meta])*
            pub struct $name;

            impl DirKind for $name {
                const NAME: &'static str = concat!(stringify!($name), "Dir");
                const LABEL: &'static str = $label;
                const COMPONENT_ID: &'static str = concat!("overseerd:dir:", $label);

                fn project_path(dirs: &ProjectDirs) -> PathBuf {
                    let resolve: fn(&ProjectDirs) -> PathBuf = $project;

                    resolve(dirs)
                }
            }
        )*
    };
}

dir_kinds! {
    /// Configuration files (`application.toml`, …).
    Config => "config", |d| d.config_dir().to_path_buf();
    /// Persistent application data.
    Data => "data", |d| d.data_dir().to_path_buf();
    /// Discardable cached data.
    Cache => "cache", |d| d.cache_dir().to_path_buf();
    /// State that should persist but is not user data (logs, history).
    State => "state",
        |d| d.state_dir().unwrap_or_else(|| d.data_dir()).to_path_buf();
    /// Runtime files (sockets, pid files); falls back to the cache dir.
    Runtime => "runtime",
        |d| d.runtime_dir().map(Path::to_path_buf).unwrap_or_else(|| d.cache_dir().to_path_buf());
    /// The system temporary directory (shared, not app-scoped).
    Tmp => "tmp", |_| std::env::temp_dir();
}

/// A resolved application directory of kind `K`, injectable by value.
///
/// Derefs to its [`Path`]. Resolution happened once at the [`DirectoriesManager`], so
/// reading a `Dir<K>` never fails; only creating it on disk ([`ensure`](Self::ensure))
/// performs I/O.
pub struct Dir<K> {
    path: PathBuf,
    _marker: PhantomData<K>,
}

impl<K> Dir<K> {
    /// Wraps an already-resolved path.
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            _marker: PhantomData,
        }
    }

    /// The resolved directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The path of `name` within this directory.
    pub fn join(&self, name: impl AsRef<Path>) -> PathBuf {
        self.path.join(name)
    }

    /// Creates the directory (and parents) on disk if absent.
    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.path)
    }
}

impl<K> Clone for Dir<K> {
    fn clone(&self) -> Self {
        Self::new(self.path.clone())
    }
}

impl<K> Deref for Dir<K> {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl<K: DirKind> Component for Dir<K> {
    const ID: &'static str = K::COMPONENT_ID;
    const NAME: &'static str = K::NAME;
    type Handle = Dir<K>;

    fn into_handle(self) -> Self::Handle {
        self
    }
}

impl<K: DirKind> Injectable for Dir<K> {
    type Target = Dir<K>;
}

/// Under `di-check`, every `Dir<K>` is framework-seeded, so it is always provided.
#[cfg(feature = "di-check")]
impl<K: DirKind> crate::descriptors::Provide<Dir<K>> for crate::descriptors::Wiring {}

/// How a [`DirectoriesManager`] resolves directories.
#[derive(Clone)]
enum Backing {
    /// Platform project directories (XDG / Known Folders).
    Project(ProjectDirs),
    /// A single root all kinds hang off, used when there is no home directory.
    Rooted(PathBuf),
}

/// Resolves and hands out the application's [`Dir<K>`] directories.
///
/// Built once (in `main` or by the daemon builder) from project metadata; cloneable
/// and injectable, so components can resolve arbitrary directory kinds on demand.
#[derive(Clone)]
pub struct DirectoriesManager {
    backing: Backing,
}

impl DirectoriesManager {
    /// Creates a new manager from any path, can be used if nothing else is possible or if default behavior
    /// is not enough. It will never fail.
    pub fn from_path(path: PathBuf) -> Self {
        Self {
            backing: Backing::Rooted(path),
        }
    }

    /// Resolves directories from project metadata (reverse-DNS `qualifier`,
    /// `organization`, `application`). `None` if no valid home directory exists.
    pub fn from_project(qualifier: &str, organization: &str, application: &str) -> Option<Self> {
        ProjectDirs::from(qualifier, organization, application).map(|project| Self {
            backing: Backing::Project(project),
        })
    }

    /// A best-effort manager for `application`: platform project dirs when available,
    /// otherwise a layout rooted under the system temp directory. Never fails.
    pub fn for_app(application: &str) -> Self {
        if let Some(manager) = Self::from_project("", "", application) {
            return manager;
        }

        let root = std::env::temp_dir().join(application);

        Self::from_path(root)
    }

    /// Resolves the directory of kind `K`.
    pub fn dir<K: DirKind>(&self) -> Dir<K> {
        let path = match &self.backing {
            Backing::Project(project) => K::project_path(project),
            Backing::Rooted(root) => K::rooted_path(root),
        };

        Dir::new(path)
    }
}

impl Component for DirectoriesManager {
    const ID: &'static str = "overseerd:directories";
    const NAME: &'static str = "DirectoriesManager";
    type Handle = DirectoriesManager;

    fn into_handle(self) -> Self::Handle {
        self
    }
}

impl Injectable for DirectoriesManager {
    type Target = DirectoriesManager;
}

/// Under `di-check`, the manager is framework-seeded, so it is always provided.
#[cfg(feature = "di-check")]
impl crate::descriptors::Provide<DirectoriesManager> for crate::descriptors::Wiring {}
