//! Config-bound and plain components: a greeting config (auto-registered), a
//! database config bound at two paths, and a by-value pool.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use overseerd::{component, config};
use serde::Deserialize;

#[allow(dead_code)]
#[config(path = "app.server")]
#[derive(Deserialize)]
pub struct AppServer {
    pub port: u16,
    pub addr: String,
}

/// Greeting configuration, deserialized from the `app.greet` subtree and injected
/// as `Cfg<Config>`. `#[config(path = "..")]` auto-registers the binding, so
/// `auto_discover` picks it up — no explicit `configs:` entry needed.
#[config(path = "app.greet")]
#[derive(Deserialize)]
pub struct Config {
    pub greeting: String,
}

/// Database connection settings. The same type is bound at two paths
/// (`app.db.reader` / `app.db.writer`) — identical shape, different usage — so it is
/// registered explicitly per path (bare `#[config]`, no baked-in path) and selected
/// at the injection site by property path.
#[config]
#[derive(Deserialize)]
pub struct DbConfig {
    pub url: String,
    pub pool_size: u16,
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
