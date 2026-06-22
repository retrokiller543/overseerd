//! Plain components: a config provided at startup, and a by-value pool.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use overseer::{Component, component};

/// Application configuration. `#[derive(Component)]` only supplies the metadata;
/// the instance is handed to the builder with `with_component`, so it is injected
/// as `Dynamic<Arc<Config>>` (runtime-provided) by anything that needs it.
#[derive(Component)]
pub struct Config {
    pub greeting: String,
}

/// A connection pool that is internally `Arc` and therefore cheap to clone, so
/// `#[component(by_value)]` stores and injects it as `Db` directly — no outer
/// `Arc<Db>`. The `#[default]` field is owned state, not an injected dependency.
#[component(by_value)]
#[derive(Clone)]
pub struct Db {
    #[default]
    queries: Arc<AtomicU64>,
}

impl Db {
    /// Records a query and returns the running total. Shared across all clones
    /// of the handle, since the counter lives behind the internal `Arc`.
    pub fn record_query(&self) -> u64 {
        self.queries.fetch_add(1, Ordering::Relaxed) + 1
    }
}
