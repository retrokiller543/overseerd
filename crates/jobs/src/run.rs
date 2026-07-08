//! The per-job driver loop and single-run execution path.
//!
//! Every job — static or dynamic — is driven by one [`drive_job`] task. The loop decides
//! *when* to fire (interval on the monotonic clock, cron on the wall clock, plus manual
//! triggers), applies pause/reschedule wake-ups, and hands each firing to [`start_run`], which
//! spawns the body so the loop never blocks. [`execute_run`] wraps a single run in its
//! [`job.run`] tracing span, enforces the overlap policy, timeout, and jitter, and records the
//! run's outcome back into the [`JobEntry`].
//!
//! [`job.run`]: crate::log::RUN_SPAN_NAME

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::time::{Instant, sleep_until};
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, error, info_span, trace};

use crate::descriptor::JobOutcome;
use crate::log::RUN_SPAN_NAME;
use crate::registry::{JobEntry, JobId, JobRunId, JobRunOutcome, JobTrigger};
use crate::schedule::{OverlapPolicy, Schedule};

#[cfg(test)]
mod tests;

/// An erased, re-runnable job body, invoked once per run with a fresh [`JobRunContext`].
///
/// Static `#[job]`s wrap their generated call (closing over a cloned `RootResolver`); dynamic
/// jobs wrap the user's closure. `Arc` + `Fn` so one runner drives every run.
pub(crate) type Runner = Arc<
    dyn Fn(JobRunContext) -> Pin<Box<dyn Future<Output = JobOutcome> + Send + 'static>>
        + Send
        + Sync,
>;

/// Structured progress a running job reports through its [`JobRunContext`]. Every field is
/// optional, so a job reports only what is meaningful to it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobProgress {
    /// A named phase (`"loading"`, `"indexing"`, …).
    pub phase: Option<String>,
    /// Work completed so far (paired with [`total`](Self::total) for a ratio).
    pub current: Option<u64>,
    /// Total work expected.
    pub total: Option<u64>,
    /// A free-form status message.
    pub message: Option<String>,
}

impl JobProgress {
    /// Progress carrying only a phase name.
    pub fn phase(phase: impl Into<String>) -> Self {
        Self {
            phase: Some(phase.into()),
            ..Self::default()
        }
    }

    /// Progress carrying only a message.
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            message: Some(message.into()),
            ..Self::default()
        }
    }

    /// A `current`/`total` counter, builder-style.
    pub fn counted(mut self, current: u64, total: u64) -> Self {
        self.current = Some(current);
        self.total = Some(total);

        self
    }

    /// Attaches a message, builder-style.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());

        self
    }
}

/// Per-run context a `#[job]` method may request as an injected parameter.
///
/// Threaded through the run rather than resolved from the DI container, because its `run_id`
/// only exists per run. Use it to report [`progress`](Self::progress) while the job executes;
/// the latest snapshot is visible through [`JobInfo::progress`](crate::JobInfo).
#[derive(Clone)]
pub struct JobRunContext {
    /// This run's identifier.
    pub run_id: JobRunId,
    /// The job this run belongs to.
    pub job_id: JobId,
    inner: Arc<RunContextInner>,
}

struct RunContextInner {
    job_name: Arc<str>,
    slot: Arc<Mutex<Option<JobProgress>>>,
}

impl JobRunContext {
    /// Builds a context sharing the entry's progress slot.
    pub(crate) fn new(
        run_id: JobRunId,
        job_id: JobId,
        job_name: Arc<str>,
        slot: Arc<Mutex<Option<JobProgress>>>,
    ) -> Self {
        Self {
            run_id,
            job_id,
            inner: Arc::new(RunContextInner { job_name, slot }),
        }
    }

    /// The job's stable name.
    pub fn job_name(&self) -> &str {
        &self.inner.job_name
    }

    /// Reports the latest progress for this run, replacing any previous snapshot. Also emits a
    /// `trace` event inside the run span so it is picked up by log capture.
    pub async fn progress(&self, progress: JobProgress) {
        trace!(
            target: "overseerd::jobs",
            phase = progress.phase.as_deref(),
            current = progress.current,
            total = progress.total,
            message = progress.message.as_deref(),
            "job progress"
        );

        *self.inner.slot.lock().expect("progress not poisoned") = Some(progress);
    }
}

/// Drives one job on its schedule until its token is cancelled, then removes it from the
/// registry via `on_exit`.
pub(crate) async fn drive_job(entry: Arc<JobEntry>, on_exit: impl FnOnce()) {
    let token = entry.token.clone();

    if entry.options().run_on_startup && !token.is_cancelled() {
        maybe_start_run(&entry, JobTrigger::Schedule, None);
    }

    let mut next = compute_next(&entry);

    loop {
        entry.set_next_run_at(next.as_wall_clock(&entry));

        tokio::select! {
            biased;

            _ = token.cancelled() => break,

            _ = entry.woken() => {
                next = compute_next(&entry);

                continue;
            }

            _ = entry.triggered() => {
                for run_id in entry.take_manual() {
                    maybe_start_run(&entry, JobTrigger::Manual, Some(run_id));
                }

                continue;
            }

            _ = next.sleep() => {
                if !entry.is_paused() {
                    maybe_start_run(&entry, JobTrigger::Schedule, None);
                }

                next = compute_next(&entry);
            }
        }
    }

    entry.mark_cancelled();
    on_exit();
}

/// The next time a job fires, in the clock domain appropriate to its schedule.
enum NextFire {
    /// A monotonic-clock instant (interval schedules).
    Interval(Instant),
    /// A wall-clock time (cron schedules).
    Cron(SystemTime),
    /// No next occurrence (zero interval, or an exhausted cron): the loop parks until woken.
    Never,
}

impl NextFire {
    /// Sleeps until this fire time.
    async fn sleep(&self) {
        match self {
            NextFire::Interval(at) => sleep_until(*at).await,

            NextFire::Cron(at) => {
                let wait = at
                    .duration_since(SystemTime::now())
                    .unwrap_or(Duration::ZERO);

                tokio::time::sleep(wait).await;
            }

            NextFire::Never => std::future::pending::<()>().await,
        }
    }

    /// The wall-clock time this fire corresponds to, for [`JobInfo::next_run_at`]. `None` when
    /// the job is paused or has no next occurrence.
    fn as_wall_clock(&self, entry: &JobEntry) -> Option<SystemTime> {
        if entry.is_paused() {
            return None;
        }

        match self {
            NextFire::Interval(at) => {
                let remaining = at.saturating_duration_since(Instant::now());

                Some(SystemTime::now() + remaining)
            }

            NextFire::Cron(at) => Some(*at),
            NextFire::Never => None,
        }
    }
}

/// Computes the next fire time from the entry's current schedule and options.
fn compute_next(entry: &JobEntry) -> NextFire {
    let schedule = entry.schedule();

    match &*schedule {
        Schedule::Every(period) => {
            if period.is_zero() {
                error!(
                    target: "overseerd::jobs",
                    job = %entry.metadata.name,
                    "interval job has a zero period; it will not run"
                );

                return NextFire::Never;
            }

            NextFire::Interval(Instant::now() + *period)
        }

        Schedule::Cron(_) => {
            let tz = entry.options().timezone();

            match schedule.next_cron_occurrence(SystemTime::now(), tz) {
                Some(at) => NextFire::Cron(at),

                None => {
                    error!(
                        target: "overseerd::jobs",
                        job = %entry.metadata.name,
                        "cron job has no next occurrence; it will not run"
                    );

                    NextFire::Never
                }
            }
        }
    }
}

/// Applies the overlap policy to a firing, starting a run unless it should be skipped/deferred.
fn maybe_start_run(entry: &Arc<JobEntry>, trigger: JobTrigger, manual_id: Option<JobRunId>) {
    let policy = entry.options().overlap;

    // Allocate the run id up front so a deferred QueueOne firing keeps the same identity (and,
    // for a manual trigger, the id `run_now` already handed back to the caller).
    let run_id = manual_id.unwrap_or_else(|| entry.next_run_id());

    if entry.active() > 0 {
        match policy {
            OverlapPolicy::Skip => {
                entry.record_skipped();

                return;
            }

            OverlapPolicy::QueueOne => {
                entry.mark_pending(run_id, trigger);

                return;
            }

            OverlapPolicy::CancelPrevious => entry.cancel_active_run(),
            OverlapPolicy::Allow => {}
        }
    }

    start_run(Arc::clone(entry), trigger, run_id);
}

/// Spawns a single run's execution task, wiring the overlap guard's in-flight count and (for
/// `CancelPrevious`) the active run token.
fn start_run(entry: Arc<JobEntry>, trigger: JobTrigger, run_id: JobRunId) {
    entry.enter_run();

    let run_token = entry.token.child_token();

    if matches!(entry.options().overlap, OverlapPolicy::CancelPrevious) {
        entry.set_run_token(run_id, run_token.clone());
    }

    tokio::spawn(execute_run(entry, trigger, run_id, run_token));
}

/// Executes one run: applies jitter, opens the run span, drives the body under the timeout /
/// cancellation, records the outcome, and starts a deferred [`QueueOne`](OverlapPolicy::QueueOne)
/// run if one accumulated.
async fn execute_run(
    entry: Arc<JobEntry>,
    trigger: JobTrigger,
    run_id: JobRunId,
    run_token: CancellationToken,
) {
    let options = entry.options();

    if let Some(jitter) = options.jitter {
        let delay = jitter_delay(jitter, run_id);

        tokio::select! {
            _ = run_token.cancelled() => {}
            _ = tokio::time::sleep(delay) => {}
        }
    }

    let started_at = SystemTime::now();
    entry.record_start(run_id, trigger, started_at);

    let cx = JobRunContext::new(
        run_id,
        entry.id,
        Arc::clone(&entry.metadata.name),
        entry.progress_slot(),
    );

    let outcome = run_body(&entry, cx, &run_token, trigger).await;

    entry.record_finish(run_id, SystemTime::now(), outcome);
    entry.clear_run_token(run_id);

    // A QueueOne firing deferred while this run was active is started now, keeping the run id
    // and trigger it was queued with.
    if let Some((pending_id, pending_trigger)) = entry.exit_run()
        && !entry.token.is_cancelled()
    {
        start_run(entry, pending_trigger, pending_id);
    }
}

/// Drives the job body inside its `job.run` span under the effective deadline and the run
/// token, normalizing the result into a [`JobRunOutcome`].
async fn run_body(
    entry: &Arc<JobEntry>,
    cx: JobRunContext,
    run_token: &CancellationToken,
    trigger: JobTrigger,
) -> JobRunOutcome {
    let span = info_span!(
        target: "overseerd::jobs",
        RUN_SPAN_NAME,
        job_id = entry.id.raw(),
        job_name = %entry.metadata.name,
        run_id = cx.run_id.raw(),
        trigger = %trigger,
    );

    let body = (entry.runner)(cx);

    let guarded = async {
        tokio::select! {
            _ = run_token.cancelled() => JobRunOutcome::Cancelled,

            outcome = body => match outcome {
                Ok(()) => JobRunOutcome::Success,
                Err(error) => JobRunOutcome::Failed(error.to_string()),
            },
        }
    }
    .instrument(span);

    match entry.options().deadline() {
        Some(deadline) => match tokio::time::timeout(deadline, guarded).await {
            Ok(outcome) => outcome,
            Err(_) => JobRunOutcome::TimedOut,
        },

        None => guarded.await,
    }
}

/// A deterministic pseudo-random jitter of up to `jitter`, seeded from the run id and the
/// current time — avoids pulling in an RNG dependency for a load-spreading nicety.
fn jitter_delay(jitter: Duration, run_id: JobRunId) -> Duration {
    let span = jitter.as_millis().max(1) as u64;

    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0)
        ^ run_id.raw().wrapping_mul(0x9E37_79B9_7F4A_7C15);

    Duration::from_millis(seed % span)
}
