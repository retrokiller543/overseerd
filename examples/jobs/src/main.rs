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
//! - **per-run log capture** via `JobLogLayer` feeding an `InMemoryJobLogStore`,
//! - **introspection** (`list_jobs`, `metrics`) and a **manual trigger** (`run_now`) from a
//!   monitor task.
//! - a named `app!` host whose setup, construction, and serving phases own the complete process
//!   lifecycle.
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
use overseerd::jobs::{
    JobLogConfig, JobLogSink, JobProgress, JobRunContext, JobScheduler, JobsPlugin, Schedule,
    configure_bootstrap_tracing, jobs,
};
use overseerd::{ConfigManager, app, component, methods};
use tracing::info;

/// Failures raised while wiring or serving the jobs application lifecycle.
#[derive(Debug, thiserror::Error)]
enum JobsApplicationError {
    /// Setup did not retain the jobs log sink for post-build scheduler wiring.
    #[error("jobs log sink is missing from the lifecycle context")]
    MissingLogSink,
    /// The jobs plugin did not seed its scheduler in the built root container.
    #[error("jobs scheduler is missing from the built application")]
    MissingScheduler,
    /// The built application failed during startup, shutdown waiting, or shutdown hooks.
    #[error(transparent)]
    Application(#[from] overseerd::AppError),
}

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

app! {
    /// Runs scheduled jobs through generated setup, build, and serve lifecycle phases.
    app JobsApplication {
        name: "jobs-example",
        protocol: overseerd::daemon::Rpc,
        managers: {
            config: ConfigManager::<Toml>::empty(),
        },
        plugins: [JobsPlugin],
        cli: {
            log: { default_value: "info,overseerd=debug" },
        },
        setup(context) {
            let mut context = context;

            // Setup contributes capture before generated bootstrap installs the tracing subscriber.
            let log_sink = configure_bootstrap_tracing(&mut context, JobLogConfig::default());

            context.insert(log_sink);

            Ok::<_, JobsApplicationError>(context)
        },
        after_build(context, app) {
            // Construction has seeded the scheduler, so runtime-only wiring belongs here.
            let log_sink = context
                .remove::<Arc<dyn JobLogSink>>()
                .ok_or(JobsApplicationError::MissingLogSink)?;
            let scheduler = app
                .container()
                .get::<JobScheduler>()
                .ok_or(JobsApplicationError::MissingScheduler)?;

            scheduler.set_log_sink(log_sink);

            // This dynamic schedule models a job loaded from an external source at runtime.
            let _handle = scheduler.schedule_named(
                "poll-upstream",
                Schedule::every(Duration::from_secs(3)),
                || async {
                    info!(target: "overseerd::example", "dynamic job fired");

                    Ok(())
                },
            );

            // Monitoring starts only after the scheduler and capture sink are fully connected.
            tokio::spawn(monitor(Arc::clone(&scheduler)));

            Ok::<_, JobsApplicationError>(app)
        },
        serve(_context, app) {
            info!(target: "overseerd::example", "daemon running — Ctrl-C to stop");

            app.run().await?;

            Ok::<(), JobsApplicationError>(())
        },
    }
}

#[tokio::main]
async fn main() -> Result<(), overseerd::CliError> {
    JobsApplication::run().await
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
