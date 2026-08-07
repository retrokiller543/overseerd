//! Homeledger configuration and database components.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use serde::Deserialize;
use upwell::{component, config, methods};

#[allow(dead_code)]
#[config(path = "homeledger.server")]
#[derive(Deserialize)]
pub struct AppServer {
    pub port: u16,
    pub addr: String,

    /// Omitted from `application.toml`, so it falls back to its templated default: the
    /// `${@runtime}` directory namespace resolves to the platform runtime dir, giving a
    /// socket path under it without hardcoding a location. The `#[serde(rename)]` proves
    /// the default keys on the *serde* name (`socket_path`), not the Rust identifier.
    #[serde(rename = "socket_path")]
    #[default = "${@runtime}/homeledger.sock"]
    pub socket: PathBuf,
}

/// Storage backend selection, demonstrating `#[config]` on an **internally-tagged enum**
/// (`tag = "kind"`) — the config picks the variant with `kind = "memory"` / `kind = "disk"`
/// (lower-cased by `rename_all`). `#[default]` marks `Memory` as the variant chosen when
/// `[homeledger.storage]` names none (or is absent). A variant-field default applies only when that
/// variant is present — `disk`'s `path` falls back to a `${@data}`-rooted location when
/// omitted, filled flat alongside the `kind` tag.
#[allow(dead_code)]
#[config(path = "homeledger.storage")]
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Storage {
    #[default]
    Memory,
    Disk {
        #[default = "${@data}/blobs"]
        path: PathBuf,
    },
}

/// Household ledger configuration, deserialized from the `homeledger.ledger` subtree and injected
/// as `Cfg<LedgerConfig>`. `#[config(path = "..")]` auto-registers the binding, so
/// `auto_discover` picks it up — no explicit `configs:` entry needed.
#[config(path = "homeledger.ledger")]
#[derive(Deserialize)]
pub struct LedgerConfig {
    pub household: String,
    pub currency: String,
}

/// Database connection settings. The same type is bound at two paths
/// (`homeledger.database.reader` / `homeledger.database.writer`) — identical shape, different usage — so it is
/// registered explicitly per path (bare `#[config]`, no baked-in path) and selected
/// at the injection site by property path.
#[config]
#[derive(Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    pub pool_size: u16,
}

/// Homeledger's in-process database pool and schema state.
#[component(by_value, factory = build_database)]
#[derive(Clone)]
pub struct Database {
    connection: Arc<AtomicUsize>,

    transactions: Arc<AtomicU64>,

    schema_version: Arc<AtomicU64>,
}

#[cfg(test)]
static DATABASE_BUILDS: AtomicUsize = AtomicUsize::new(0);

async fn build_database() -> Database {
    #[cfg(test)]
    DATABASE_BUILDS.fetch_add(1, Ordering::SeqCst);

    Database {
        connection: Arc::new(AtomicUsize::new(0)),
        transactions: Arc::new(AtomicU64::new(0)),
        schema_version: Arc::new(AtomicU64::new(0)),
    }
}

#[cfg(test)]
pub(crate) fn reset_database_builds() {
    DATABASE_BUILDS.store(0, Ordering::SeqCst);
}

#[cfg(test)]
pub(crate) fn database_builds() -> usize {
    DATABASE_BUILDS.load(Ordering::SeqCst)
}

impl Database {
    pub fn create_connection(&self) -> DatabaseConnection {
        let id = self.connection.fetch_add(1, Ordering::Relaxed);
        let transactions = self.transactions.clone();

        DatabaseConnection::new(id, transactions)
    }

    /// Verifies the schema and returns its current version.
    pub fn verify_schema(&self) -> u64 {
        self.schema_version.load(Ordering::Relaxed)
    }

    /// Applies pending migrations and returns the resulting schema version.
    pub fn migrate(&self, target: u64) -> u64 {
        self.schema_version.store(target, Ordering::Relaxed);

        target
    }
}

/// A request-scoped database connection used by ledger RPC handlers.
#[component(scope = upwell::daemon::Request)]
pub struct DatabaseConnection {
    #[default]
    id: usize,
    #[default]
    transactions: Arc<AtomicU64>,
}

impl DatabaseConnection {
    pub fn new(id: usize, transactions: Arc<AtomicU64>) -> Self {
        Self { id, transactions }
    }

    #[tracing::instrument(skip(self), fields(connection_id = self.id))]
    pub fn record_transaction(&self) -> u64 {
        self.transactions.fetch_add(1, Ordering::Relaxed) + 1
    }
}

#[methods]
impl DatabaseConnection {
    #[init]
    pub async fn init(database: Database) -> Self {
        database.create_connection()
    }
}
