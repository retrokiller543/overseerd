//! # Overseerd Jobs
//!
//! An optional job scheduler for Overseerd, integrated as a non-protocol
//! [`Plugin`](overseerd_app::Plugin). Mark an `async` method on a `#[service]`/`#[component]`
//! with `#[job(every = "..")]` or `#[job(cron = "..")]` and register [`JobsPlugin`]: the
//! framework spawns a supervised background loop that runs the method on schedule, resolving
//! its `&self` receiver and any `Inject<_>` parameters from the DI container on each run.
//!
//! ```ignore
//! use overseerd::jobs::JobsPlugin;
//!
//! #[service]
//! struct Reaper { db: Dep<Db> }
//!
//! #[handlers]
//! impl Reaper {
//!     #[job(every = "30s")]
//!     async fn sweep(&self) { self.db.cleanup().await; }
//!
//!     #[job(cron = "@hourly")]
//!     async fn report(&self, Inject(metrics): Inject<Dep<Metrics>>) { metrics.flush().await; }
//! }
//!
//! app.plugin(JobsPlugin::default())
//! ```
//!
//! The scheduler is a hand-rolled framework singleton driven by a `Startup` hook (spawn) and
//! its `Drop` (graceful cancel); jobs are discovered at link time, so registering the plugin
//! is all that is required. Schedules parse at startup — an invalid one fails startup.

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
