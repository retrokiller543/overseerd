//! Manual configuration reloading.
//!
//! [`ConfigReloader`] re-reads the [`ConfigManager`]'s sources, diffs each binding's
//! merged subtree against the live tree, and swaps **only** the bindings whose source
//! actually changed — so an unchanged value is never re-published (and, in a later
//! phase, never fires a reload hook). The swap is two-phase: every changed binding is
//! re-deserialized first (fallible), and only if all succeed are the new values
//! committed into their shared [`Live`](crate::descriptors::Live) slots, so a reload
//! is all-or-nothing.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use overseerd_config::ConfigValue;

use crate::descriptors::{BoxedComponent, Injectable};

use super::{Cfg, ConfigError, ConfigManager, ConfigProperties};

/// Errors from a configuration reload. On any error the live values are left
/// untouched — a reload commits only when every changed binding re-binds cleanly.
#[derive(Debug, thiserror::Error)]
pub enum ConfigReloadError {
    /// Re-reading or re-merging the config sources failed.
    #[error("failed to re-read configuration: {0}")]
    Load(#[source] ConfigError),

    /// A changed binding failed to deserialize from the new tree.
    #[error("failed to re-bind '{path}' as {type_name} during reload: {source}")]
    Bind {
        path: String,
        type_name: &'static str,
        #[source]
        source: ConfigError,
    },
}

/// One binding changed by a reload.
#[derive(Debug, Clone)]
pub struct ChangedBinding {
    pub path: String,
    pub type_name: &'static str,
}

/// The outcome of a successful reload.
#[derive(Debug, Clone)]
pub struct ConfigReloadReport {
    /// Monotonic counter incremented on every successful reload.
    pub generation: u64,
    /// The bindings whose source changed and were re-published. Empty when nothing
    /// changed.
    pub changed: Vec<ChangedBinding>,
}

/// A re-deserialized value ready to publish into its slot. Produced in the reload's
/// prepare step; run in the commit step once every changed binding has prepared.
pub struct PreparedSwap {
    commit: Box<dyn FnOnce() + Send>,
}

/// A config binding the reloader can re-bind: it knows its path and how to
/// re-deserialize its type from a re-read tree and swap its live slot. Object-safe so
/// the reloader holds a `Vec<Box<dyn ReloadableConfig>>` across all bound types.
pub trait ReloadableConfig: Send + Sync {
    /// The property path this binding was bound at.
    fn path(&self) -> &str;

    /// The bound type's display name, for reports and errors.
    fn type_name(&self) -> &'static str;

    /// Re-deserializes the binding from `new_root` and returns a committable swap.
    /// Fallible — a deserialize error aborts the whole reload before any commit.
    fn prepare(
        &self,
        manager: &ConfigManager,
        new_root: &ConfigValue,
    ) -> Result<PreparedSwap, ConfigError>;
}

/// The reloadable record for one `Cfg<T>` binding: a clone of the injected handle
/// (sharing its `Live` slot) plus the property path.
pub(crate) struct ConfigSlot<T> {
    cfg: Cfg<T>,
    path: String,
}

impl<T: ConfigProperties> ConfigSlot<T> {
    /// Recovers a reloadable slot from a freshly bound config seed, sharing its live
    /// cell. Returns `None` if the seed does not hold a `Cfg<T>`.
    pub(crate) fn from_seed(seed: &BoxedComponent, path: &str) -> Option<Box<dyn ReloadableConfig>> {
        let cfg = seed.value.downcast_ref::<Cfg<T>>()?.clone();

        Some(Box::new(ConfigSlot {
            cfg,
            path: path.to_string(),
        }))
    }
}

impl<T: ConfigProperties> ReloadableConfig for ConfigSlot<T> {
    fn path(&self) -> &str {
        &self.path
    }

    fn type_name(&self) -> &'static str {
        T::NAME
    }

    fn prepare(
        &self,
        manager: &ConfigManager,
        new_root: &ConfigValue,
    ) -> Result<PreparedSwap, ConfigError> {
        let value: T = manager.get_config_in::<T>(new_root, &self.path)?;
        let replacement = Arc::new(value);
        let cfg = self.cfg.clone();

        Ok(PreparedSwap {
            commit: Box::new(move || cfg.replace(replacement)),
        })
    }
}

/// A cheap, cloneable, injectable handle that re-reads configuration on demand.
///
/// Seeded by the daemon as a framework singleton, so any component or handler can
/// inject it (`reloader: ConfigReloader`) and trigger a reload. Always available;
/// signal- and file-watch-driven reloads (configured on the [`ConfigManager`]) build
/// on the same [`reload`](Self::reload) entry point.
#[derive(Clone)]
pub struct ConfigReloader {
    inner: Arc<ReloaderInner>,
}

struct ReloaderInner {
    manager: Mutex<ConfigManager>,
    slots: Vec<Box<dyn ReloadableConfig>>,
    generation: AtomicU64,
}

impl ConfigReloader {
    /// Builds a reloader over the manager and the reloadable slots of every bound
    /// config (sharing their live cells).
    pub(crate) fn new(manager: ConfigManager, slots: Vec<Box<dyn ReloadableConfig>>) -> Self {
        Self {
            inner: Arc::new(ReloaderInner {
                manager: Mutex::new(manager),
                slots,
                generation: AtomicU64::new(0),
            }),
        }
    }

    /// The number of successful reloads so far (the current generation).
    pub fn generation(&self) -> u64 {
        self.inner.generation.load(Ordering::SeqCst)
    }

    /// Re-reads the config sources and publishes the changed bindings.
    ///
    /// Re-merges all sources in their original order (so profile precedence is
    /// preserved), diffs each binding's subtree, re-deserializes only the changed
    /// ones, and — if all succeed — commits them into their shared slots. On any
    /// failure nothing is published and the live values are untouched.
    pub async fn reload(&self) -> Result<ConfigReloadReport, ConfigReloadError> {
        let mut manager = self
            .inner
            .manager
            .lock()
            .expect("config manager mutex poisoned");

        let new_root = manager.reread().map_err(ConfigReloadError::Load)?;
        let mut prepared = Vec::new();
        let mut changed = Vec::new();

        for slot in &self.inner.slots {
            if manager.subtree(slot.path()) == new_root.get_path(slot.path()) {
                continue;
            }

            let swap = slot.prepare(&manager, &new_root).map_err(|source| {
                ConfigReloadError::Bind {
                    path: slot.path().to_string(),
                    type_name: slot.type_name(),
                    source,
                }
            })?;

            prepared.push(swap);
            changed.push(ChangedBinding {
                path: slot.path().to_string(),
                type_name: slot.type_name(),
            });
        }

        for swap in prepared {
            (swap.commit)();
        }

        manager.adopt(new_root);

        let generation = self.inner.generation.fetch_add(1, Ordering::SeqCst) + 1;

        Ok(ConfigReloadReport {
            generation,
            changed,
        })
    }
}

/// The stable component id of the seeded [`ConfigReloader`] singleton.
pub const CONFIG_RELOADER_ID: &str = "overseerd:config-reloader";

/// The display name of the seeded [`ConfigReloader`] singleton.
pub const CONFIG_RELOADER_NAME: &str = "ConfigReloader";

impl crate::descriptors::Component for ConfigReloader {
    const ID: &'static str = CONFIG_RELOADER_ID;
    const NAME: &'static str = CONFIG_RELOADER_NAME;
    type Handle = ConfigReloader;

    fn into_handle(self) -> Self::Handle {
        self
    }
}

impl Injectable for ConfigReloader {
    type Target = ConfigReloader;
    type Stored = Self;

    fn into_stored(self) -> Self {
        self
    }

    fn from_stored(stored: &Self) -> Self {
        stored.clone()
    }
}

/// Under `di-check`, the reloader is framework-seeded, so it is always provided.
#[cfg(feature = "di-check")]
impl crate::descriptors::Provide<ConfigReloader> for crate::descriptors::Wiring {}
