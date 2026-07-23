//! A minimal Overseerd daemon that runs scheduled **jobs**, showing the observability and
//! control surface of `overseerd-jobs`.
//!
//! It demonstrates:
//!
//! - a `#[job(every = "..")]` interval job (`Heartbeat::tick`),
//! - a `#[job]` that **injects a dependency** and requests the per-run [`JobRunContext`] to
//!   report **progress** (`Heartbeat::announce`),
//! - `#[job(..)]` **execution options** — `run_on_startup`, `timeout`, `overlap` — on
//!   `Heartbeat::rebuild_index`,
//! - a `#[job(cron = "..")]` cron job (`Heartbeat::hourly`),
//! - a **named dynamic** job scheduled at run time (`JobScheduler::schedule_named`),
//! - **per-run log capture** via a [`JobLogLayer`] feeding an [`InMemoryJobLogStore`],
//! - **introspection** (`list_jobs`, `metrics`) and a **manual trigger** (`run_now`) from a
//!   monitor task.
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

use overseerd::config::Toml;
use overseerd::daemon::App;
use overseerd::jobs::{
    JobLogConfig, JobProgress, JobRunContext, JobScheduler, JobsPlugin, Schedule, init_tracing,
    jobs,
};
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

/// The job host: an internal beat counter (`#[default]`, so it is not injected) plus several
/// scheduled methods covering interval, cron, injected-dependency, progress, and options.
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

    /// Fires every five seconds, injects `Arc<Greeter>`, and reports progress through the
    /// per-run context. A slow previous run is cancelled (`overlap = CancelPrevious`) and any
    /// run is capped at four seconds (`timeout`).
    #[job(every = "5s", overlap = CancelPrevious, timeout = "4s")]
    async fn announce(&self, greeter: Arc<Greeter>, cx: JobRunContext) {
        cx.progress(JobProgress::phase("announcing")).await;

        info!(target: "overseerd::example", message = greeter.message(), "announce");

        cx.progress(JobProgress::message("done").counted(1, 1))
            .await;
    }

    /// Runs once immediately on startup, then every ten seconds; reports staged progress.
    #[job(every = "10s", run_on_startup)]
    async fn rebuild_index(&self, cx: JobRunContext) {
        for (done, phase) in ["loading", "indexing", "flushing"].iter().enumerate() {
            cx.progress(JobProgress::phase(*phase).counted(done as u64 + 1, 3))
                .await;

            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        info!(target: "overseerd::example", "index rebuilt");
    }

    /// Fires at the top of every hour, via a cron nickname.
    #[job(cron = "@hourly")]
    async fn hourly(&self) {
        info!(target: "overseerd::example", "hourly cron job fired");
    }
}

#[tokio::main]
async fn main() -> overseerd::daemon::Result<()> {
    // Per-run log capture, wired through the jobs-aware `init_tracing`: it installs the usual
    // framework subscriber and layers a bounded in-memory capture sink onto it, driven by
    // config. The returned sink is handed to the scheduler below. (`init_tracing` returns a
    // no-op sink instead when `JobLogConfig::enabled` is false.)
    let logging = LoggingConfig::new("info,overseerd=debug");
    let log_sink = init_tracing(&logging, JobLogConfig::default()).expect("install tracing");

    // Bind no config files — this example needs none.
    let config = ConfigManager::<Toml>::empty();

    let app = App::builder("jobs-example")
        .config_source(config)
        .auto_discover()
        .plugin(JobsPlugin)
        .build()
        .await?;

    let scheduler = app
        .container()
        .get::<JobScheduler>()
        .expect("the jobs plugin seeds the scheduler");

    // Route captured job logs into the sink `init_tracing` created (default is the no-op sink).
    scheduler.set_log_sink(log_sink);

    // A named dynamic job, as if its schedule had just been read from a database. The returned
    // handle could later `.cancel()` it; here we keep it for the process lifetime.
    let _handle = scheduler.schedule_named(
        "poll-upstream",
        Schedule::every(Duration::from_secs(3)),
        || async {
            info!(target: "overseerd::example", "dynamic job fired");

            Ok(())
        },
    );

    // A monitor task: periodically logs the aggregate metrics and per-job state, and manually
    // triggers the `announce` job to show `run_now` and log capture working together.
    tokio::spawn(monitor(Arc::clone(&scheduler)));

    info!(target: "overseerd::example", "daemon running — Ctrl-C to stop");

    app.run().await?;

    Ok(())
}

/// Periodically reports scheduler state and demonstrates a manual trigger plus log lookup.
async fn monitor(scheduler: Arc<JobScheduler>) {
    tokio::time::sleep(Duration::from_secs(4)).await;

    loop {
        let metrics = scheduler.metrics();

        info!(
            target: "overseerd::example",
            jobs = metrics.jobs_scheduled,
            active = metrics.active_runs,
            completed = metrics.completed_runs,
            failed = metrics.failed_runs,
            "scheduler metrics"
        );

        for info in scheduler.list_jobs() {
            info!(
                target: "overseerd::example",
                job = %info.name,
                state = ?info.state,
                runs = info.run_count,
                "job state"
            );
        }

        // Manually trigger the announce job and read back what it logged.
        let announce = scheduler
            .list_jobs()
            .into_iter()
            .find(|j| j.name.ends_with("announce"));

        if let Some(announce) = announce
            && let Ok(run_id) = scheduler.run_now(announce.id).await
        {
            tokio::time::sleep(Duration::from_millis(100)).await;

            let records = scheduler.log_records(run_id, 16).await;

            info!(
                target: "overseerd::example",
                run = %run_id,
                captured = records.len(),
                "captured logs for manual run"
            );
        }

        tokio::time::sleep(Duration::from_secs(8)).await;
    }
}
