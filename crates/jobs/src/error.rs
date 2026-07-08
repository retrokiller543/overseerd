//! Errors from the scheduler's control APIs.

use crate::registry::JobId;
use crate::schedule::ScheduleError;

/// An error from a [`JobScheduler`](crate::JobScheduler) control operation
/// (`run_now`, `pause`, `reschedule`, …).
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    /// No job with the given id is registered (it may have been cancelled).
    #[error("no job with id {0}")]
    UnknownJob(JobId),

    /// No job with the given name is registered.
    #[error("no job named '{0}'")]
    UnknownName(String),

    /// A [`reschedule`](crate::JobScheduler::reschedule) was given an invalid schedule.
    #[error(transparent)]
    Schedule(#[from] ScheduleError),
}
