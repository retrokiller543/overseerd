//! Config-bound and plain components: a greeting config (auto-registered), a
//! database config bound at two paths, a by-value pool, and a health check.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use overseerd::{ConfigProperties, HealthCheck, HealthCheckFuture, HealthStatus, component};
use serde::Deserialize;

#[allow(dead_code)]
#[derive(ConfigProperties, Deserialize)]
#[config(path = "app.server")]
pub struct AppServer {
    pub port: u16,
    pub addr: String,
}

/// Greeting configuration, deserialized from the `app.greet` subtree and injected
/// as `Cfg<Config>`. `#[config(path = "..")]` auto-registers the binding, so
/// `auto_discover` picks it up — no explicit `configs:` entry needed.
#[derive(ConfigProperties, Deserialize)]
#[config(path = "app.greet")]
pub struct Config {
    pub greeting: String,
}

/// Database connection settings. The same type is bound at two paths
/// (`app.db.reader` / `app.db.writer`) — identical shape, different usage — so it is
/// registered explicitly per path (no baked-in `#[config(path)]`) and selected at
/// the injection site by property path.
#[derive(ConfigProperties, Deserialize)]
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

/// A component that monitors database reachability and reports health.
///
/// Registered as a normal singleton component *and* as a `dyn HealthCheck`
/// provider via `provide = dyn HealthCheck`. The framework collects it
/// automatically — no extra registration needed.
///
/// When running under systemd with `WatchdogSec` set the framework polls this
/// on the watchdog interval and suppresses the ping if `check()` returns
/// `Unhealthy`, causing the service manager to restart the process.
#[component(provide = dyn HealthCheck)]
pub struct DbHealthCheck {
    #[default]
    healthy: Arc<AtomicBool>,
}

impl DbHealthCheck {
    /// Simulates marking the database as unavailable (for testing).
    #[allow(dead_code)]
    pub fn set_healthy(&self, val: bool) {
        self.healthy.store(val, Ordering::Relaxed);
    }
}

impl HealthCheck for DbHealthCheck {
    fn name(&self) -> &str {
        "database"
    }

    fn check(&self) -> HealthCheckFuture {
        let healthy = Arc::clone(&self.healthy);
        Box::pin(async move {
            if healthy.load(Ordering::Relaxed) {
                HealthStatus::Healthy
            } else {
                HealthStatus::Unhealthy
            }
        })
    }
}
