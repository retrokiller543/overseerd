//! # Overseerd Jobs
//!
//! A job scheduler for Overseerd, integrated as a non-protocol
//! [`Plugin`](overseerd_app::Plugin). Mark an `async` method on a `#[component]` with
//! `#[job(every = "..")]` or `#[job(cron = "..")]` inside a [`#[jobs]`](macro@jobs) impl block,
//! register [`JobsPlugin`], and the framework spawns a supervised background loop that runs the
//! method on schedule — resolving its `&self` receiver and any dependency parameters from the
//! DI container on each run.
//!
//! ```ignore
//! use overseerd::jobs::{JobsPlugin, jobs};
//! use overseerd::{component, Dep};
//!
//! #[component]
//! struct Reaper { db: Dep<Db> }
//!
//! #[jobs]
//! impl Reaper {
//!     // Reaches its dependencies through `&self`.
//!     #[job(every = "30s")]
//!     async fn sweep(&self) { self.db.snapshot().cleanup().await; }
//!
//!     // Extra parameters are resolved from the container per run — the shapes an
//!     // `#[init]` constructor takes (`Arc<T>`, `Dep<T>`, `Cfg<T>`, …), no wrapper needed.
//!     #[job(cron = "@hourly")]
//!     async fn report(&self, metrics: Dep<Metrics>) { metrics.snapshot().flush().await; }
//! }
//!
//! app.plugin(JobsPlugin);
//! ```
//!
//! ## Schedules
//!
//! - `every = ".."` — a fixed interval parsed by [`humantime`] (`"30s"`, `"5m"`, `"1h 30m"`,
//!   `"500ms"`), run on the monotonic clock. `months`/`years` are fixed approximations, so an
//!   interval is a constant wall-time gap, not a calendar cadence.
//! - `cron = ".."` — a cron expression or `@`-nickname (`"0 3 * * *"`, `"@hourly"`), run on the
//!   wall clock.
//!
//! Schedules parse at startup — an invalid one fails startup rather than silently never firing.
//! Each job runs on its own cancellation token, and a run is awaited sequentially, so a body
//! that outlasts its period simply skips the ticks it missed (the built-in overlap guard).
//!
//! ## Dynamic jobs
//!
//! Jobs whose existence or cadence is only known at run time (e.g. loaded from a database) are
//! added through [`JobScheduler::schedule`]. The scheduler is an injectable singleton, so any
//! component can take `Arc<JobScheduler>` and schedule work; the returned [`JobHandle`] cancels
//! (un-registers) it.
//!
//! ```ignore
//! let handle = scheduler.schedule(Schedule::interval("5m")?, || async {
//!     poll_upstream().await?;
//!     Ok(())
//! });
//! // later …
//! handle.cancel();
//! ```
//!
//! ## Standalone job runner
//!
//! The scheduler needs no network protocol. Because [`App::run`](overseerd_app::App::run) runs
//! the startup hooks (which spawn the jobs) and then simply waits for shutdown — without serving
//! any endpoint — an app built with [`JobsPlugin`] and driven by `run()` (not `serve()`) is a
//! dedicated **scheduler process** with no request surface: a cron/worker daemon. Pair the
//! `jobs` feature with any protocol (its plugin only supplies the `App` type) and never call
//! `serve`; see `examples/jobs`.
//!
//! The scheduler is a framework singleton driven by a `Startup` hook (spawn) and its `Drop`
//! (graceful cancel of every loop); static jobs are discovered at link time, so registering the
//! plugin is all that is required.

pub mod descriptor;
pub mod plugin;
pub mod schedule;
pub mod scheduler;

pub use descriptor::{JOBS, JobCall, JobDescriptor, JobOutcome};
pub use plugin::JobsPlugin;
pub use schedule::{Schedule, ScheduleError, ScheduleKind};
pub use scheduler::{JobHandle, JobId, JobScheduler};

/// The `#[jobs]` impl-block macro and its `#[job]` method marker, owned by
/// `overseerd-jobs-macros` and re-exported here (the plugin crate owns its macros).
pub use overseerd_jobs_macros::{job, jobs};

/// Re-exported so `#[job]`-generated code can reach the `#[distributed_slice]` attribute
/// through a stable path (`overseerd::jobs::linkme`).
#[doc(hidden)]
pub use linkme;
