//! Backend-agnostic operational observation.
//!
//! `overseerd-jobs` deliberately ships no metrics client and no health policy. Instead it
//! exposes the *state* an application needs to feed its own system — [`metrics`], OpenTelemetry,
//! a health endpoint — through [`JobScheduler::metrics`](crate::JobScheduler::metrics) (an
//! aggregate [`JobMetrics`] snapshot) and [`JobScheduler::list_jobs`](crate::JobScheduler::list_jobs)
//! (per-job detail). Whether a failed or stale job makes the process unhealthy is the
//! application's call, so this module offers helpers ([`JobInfo::schedule_lag`],
//! [`JobScheduler::stale_jobs`](crate::JobScheduler::stale_jobs)) rather than a verdict.

use std::time::{Duration, SystemTime};

use crate::registry::JobInfo;

/// An aggregate snapshot of scheduler activity, cheap to sample on a metrics tick.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobMetrics {
    /// Jobs currently registered with the scheduler.
    pub jobs_scheduled: usize,
    /// Jobs whose scheduling is paused.
    pub paused_jobs: usize,
    /// Runs currently in flight across all jobs.
    pub active_runs: usize,
    /// Runs that have completed (successfully or not) across all jobs.
    pub completed_runs: u64,
    /// Runs that failed or timed out across all jobs.
    pub failed_runs: u64,
    /// Firings skipped by the overlap guard across all jobs.
    pub skipped_ticks: u64,
}

impl JobInfo {
    /// How overdue the next scheduled run is: the time elapsed past [`next_run_at`], or `None`
    /// if the job is paused, has no next occurrence, or is not yet due.
    ///
    /// [`next_run_at`]: JobInfo::next_run_at
    pub fn schedule_lag(&self) -> Option<Duration> {
        let next = self.next_run_at?;

        SystemTime::now().duration_since(next).ok()
    }

    /// The wall-clock duration of the most recent completed run, if any.
    pub fn last_run_duration(&self) -> Option<Duration> {
        self.last_run.as_ref()?.duration()
    }

    /// Whether the job is overdue by more than `threshold` — a building block for an
    /// application-defined staleness policy.
    pub fn is_stale(&self, threshold: Duration) -> bool {
        self.schedule_lag().is_some_and(|lag| lag > threshold)
    }
}
