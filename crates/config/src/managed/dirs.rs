//! The `${@<label>}` directory namespace resolver.
//!
//! `overseerd-dirs` is config-agnostic: it exposes its known directories as plain
//! `(label, path)` data via [`DirectoriesManager::entries`]. This module turns that data
//! into a [`config::Resolver`](crate::Resolver), so a config value like
//! `"${@runtime}/app.sock"` resolves against the platform's directories. config owns the
//! `Resolver` trait, so it provides the impl for dirs' data — config never names the
//! individual `Dir` kind types.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;

use overseerd_dirs::DirectoriesManager;

use crate::Resolver;

/// A [`config::Resolver`](crate::Resolver) backing the `@` directory namespace.
///
/// Answers a placeholder keyed `@<label>` (one of `@config`, `@data`, `@cache`, `@state`,
/// `@runtime`, `@tmp`) with the pre-resolved path for that directory.
pub struct DirectoriesResolver {
    entries: HashMap<&'static str, PathBuf>,
}

impl DirectoriesResolver {
    /// Builds a resolver from a manager's directory list. Pre-resolves every kind once into
    /// a label→path map, so the resolver is a cheap O(1) lookup free of directory logic.
    pub fn from_manager(directories: &DirectoriesManager) -> Self {
        Self {
            entries: directories.entries().into_iter().collect(),
        }
    }
}

impl Resolver for DirectoriesResolver {
    fn resolve(&self, key: &str) -> Option<Cow<'_, str>> {
        let label = key.strip_prefix('@')?;

        let path = self.entries.get(label)?;

        Some(Cow::Owned(path.to_string_lossy().into_owned()))
    }
}
