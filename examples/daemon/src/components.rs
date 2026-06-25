//! Config-bound and plain components: a greeting config (auto-registered), a
//! database config bound at two paths, and a by-value pool.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use overseerd::{component, config, methods};
use serde::Deserialize;

#[allow(dead_code)]
#[config(path = "app.server")]
#[derive(Deserialize)]
pub struct AppServer {
    pub port: u16,
    pub addr: String,

    /// Omitted from `application.toml`, so it falls back to its templated default: the
    /// `${@runtime}` directory namespace resolves to the platform runtime dir, giving a
    /// socket path under it without hardcoding a location.
    #[default = "${@runtime}/example-daemon.sock"]
    pub socket: PathBuf,
}

/// Storage backend selection, demonstrating `#[config]` on an **enum**. Externally
/// tagged: the config selects the variant by key (`Memory` or `Disk`). A default on a
/// variant field applies only when that variant is the one present — here `Disk`'s
/// `path` falls back to a `${@data}`-rooted location when omitted.
#[allow(dead_code)]
#[config(path = "app.storage")]
#[derive(Deserialize)]
pub enum Storage {
    Memory,
    Disk {
        #[default = "${@data}/blobs"]
        path: PathBuf,
    },
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
    connection: Arc<AtomicUsize>,

    #[default]
    queries: Arc<AtomicU64>,
}

impl Db {
    pub fn create_connection(&self) -> DbConnection {
        let id = self.connection.fetch_add(1, Ordering::Relaxed);
        let queries = self.queries.clone();

        DbConnection::new(id, queries)
    }

    /// Records a query and returns the running total. Shared across all clones
    /// of the handle, since the counter lives behind the internal `Arc`.
    pub fn record_query(&self) -> u64 {
        self.queries.fetch_add(1, Ordering::Relaxed) + 1
    }
}

#[component(scope = request, by_value)]
#[derive(Clone)]
pub struct DbConnection {
    #[default]
    id: usize,
    #[default]
    queries: Arc<AtomicU64>,
}

impl DbConnection {
    pub fn new(id: usize, queries: Arc<AtomicU64>) -> Self {
        Self { id, queries }
    }

    #[tracing::instrument(skip(self), fields(connection_id = self.id))]
    pub fn record_query(&self) -> u64 {
        self.queries.fetch_add(1, Ordering::Relaxed) + 1
    }
}

#[methods]
impl DbConnection {
    #[init]
    pub async fn init(db: Db) -> Self {
        db.create_connection()
    }
}
