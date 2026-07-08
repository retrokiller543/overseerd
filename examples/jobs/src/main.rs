//! A minimal Overseerd daemon that runs scheduled **jobs**.
//!
//! It shows all three ways jobs reach the scheduler:
//!
//! - a `#[job(every = "..")]` interval job (`Heartbeat::tick`),
//! - a `#[job]` that **injects a dependency** per run (`Heartbeat::announce`),
//! - a `#[job(cron = "..")]` cron job (`Heartbeat::hourly`),
//! - a **dynamic** job scheduled at run time through `JobScheduler::schedule` (as a
//!   database-driven daemon would), returning a `JobHandle` that can cancel it.
//!
//! Run it and watch the `overseerd::example` / `overseerd::jobs` log lines:
//!
//! ```text
//! cargo run -p overseerd-example-jobs
//! ```
//!
//! Press Ctrl-C to shut down — the scheduler cancels every loop on the way out.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use overseerd::builtins::init_tracing;
use overseerd::config::Toml;
use overseerd::daemon::App;
use overseerd::jobs::{JobScheduler, JobsPlugin, Schedule, jobs};
use overseerd::{ConfigManager, LoggingConfig, component, methods};
use tracing::info;

/// A dependency a job resolves per run, proving `#[job]` methods can inject like constructors.
/// `#[default]` on the field satisfies the (unused) field-injection factory; the real value
/// comes from `#[init]`.
#[component]
struct Greeter {
    #[default]
    message: String,
}

#[methods]
impl Greeter {
    #[init]
    fn new() -> Self {
        Self {
            message: "jobs are running".to_string(),
        }
    }

    fn message(&self) -> &str {
        &self.message
    }
}

/// The job host: an internal beat counter (`#[default]`, so it is not injected) plus three
/// scheduled methods.
#[component]
struct Heartbeat {
    #[default]
    beats: AtomicU64,
}

#[jobs]
impl Heartbeat {
    /// Fires every two seconds; reaches its state through `&self`.
    #[job(every = "2s")]
    async fn tick(&self) {
        let beat = self.beats.fetch_add(1, Ordering::Relaxed) + 1;

        info!(target: "overseerd::example", beat, "heartbeat tick");
    }

    /// Fires every five seconds and injects `Arc<Greeter>` from the container per run.
    #[job(every = "5s")]
    async fn announce(&self, greeter: Arc<Greeter>) {
        info!(target: "overseerd::example", message = greeter.message(), "announce");
    }

    /// Fires at the top of every hour, via a cron nickname.
    #[job(cron = "@hourly")]
    async fn hourly(&self) {
        info!(target: "overseerd::example", "hourly cron job fired");
    }
}

#[tokio::main]
async fn main() -> overseerd::daemon::Result<()> {
    // Bind no config files — this example needs none. `get_config` still applies the
    // `LoggingConfig` defaults, so logging works out of the box.
    let config = ConfigManager::<Toml>::empty();
    init_tracing(&LoggingConfig {
        level: "info,overseerd=trace".to_string(),
        format: "full".to_string(),
        ansi: true,
    })
    .ok();

    let app = App::builder("jobs-example")
        .config_source(config)
        .auto_discover()
        .plugin(JobsPlugin)
        .build()
        .await?;

    // A dynamic job, as if its schedule had just been read from a database. The returned
    // handle could later `.cancel()` it; here we keep it for the process lifetime.
    let scheduler = app
        .container()
        .get::<JobScheduler>()
        .expect("the jobs plugin seeds the scheduler");

    let _handle = scheduler.schedule(Schedule::every(Duration::from_secs(3)), || async {
        info!(target: "overseerd::example", "dynamic job fired");

        Ok(())
    });

    info!(target: "overseerd::example", "daemon running — Ctrl-C to stop");

    app.run().await?;

    Ok(())
}
