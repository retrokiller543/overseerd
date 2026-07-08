//! The job registry: the single source of truth for every scheduled job, static or dynamic.
//!
//! The scheduler used to keep only `JobId -> CancellationToken`, enough to cancel but blind to
//! everything else. [`JobRegistry`] instead stores an [`Arc<JobEntry>`] per job — its metadata,
//! live [`JobState`], schedule, run counters, and recent [`JobRunSummary`]s — shared between the
//! public introspection API and the background loop driving the job. Static `#[job]`s and
//! dynamic jobs register through the same path, so introspection never cares where a job came
//! from.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::run::{JobProgress, Runner};
use crate::schedule::{JobOptions, Schedule, ScheduleInfo};

#[cfg(test)]
mod tests;

/// How many recent [`JobRunSummary`]s each job retains for [`recent_runs`](crate::JobScheduler::recent_runs).
const RECENT_RUNS_CAP: usize = 32;

/// An opaque identifier for a scheduled job, unique within one scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(u64);

impl JobId {
    /// Reconstructs a [`JobId`] from its raw value (used by the log capture layer, which reads
    /// the id back off a span field).
    pub(crate) fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw numeric value, for logging and span fields.
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An opaque identifier for a single job run, unique within one scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobRunId(u64);

impl JobRunId {
    /// Reconstructs a [`JobRunId`] from its raw value (used by the log capture layer).
    pub(crate) fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw numeric value, for logging and span fields.
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for JobRunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The lifecycle state of a scheduled job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobState {
    /// Waiting for its next occurrence.
    Scheduled,
    /// A run is in progress (the id of the most recent active run).
    Running { run_id: JobRunId },
    /// Scheduling is suspended; no new runs start until resumed.
    Paused,
    /// Cancellation has been requested; the loop is winding down.
    Cancelling,
    /// The job has been cancelled and removed from the scheduler.
    Cancelled,
}

/// What caused a run to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobTrigger {
    /// The job's schedule fired.
    Schedule,
    /// An operator (or test) requested the run via
    /// [`run_now`](crate::JobScheduler::run_now) / [`run_named`](crate::JobScheduler::run_named).
    Manual,
}

impl std::fmt::Display for JobTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobTrigger::Schedule => f.write_str("schedule"),
            JobTrigger::Manual => f.write_str("manual"),
        }
    }
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobRunOutcome {
    /// Still running.
    Running,
    /// Completed successfully.
    Success,
    /// The job body returned an error (its rendered message).
    Failed(String),
    /// Aborted by the configured timeout / max-runtime.
    TimedOut,
    /// Cancelled before it finished (job cancelled, or overlap `CancelPrevious`).
    Cancelled,
}

/// A summary of one job run, retained in the registry's recent-runs buffer.
#[derive(Debug, Clone)]
pub struct JobRunSummary {
    /// The run's identifier.
    pub run_id: JobRunId,
    /// What started the run.
    pub trigger: JobTrigger,
    /// When the run started.
    pub started_at: SystemTime,
    /// When the run finished, or `None` while it is still running.
    pub finished_at: Option<SystemTime>,
    /// How the run ended (or [`Running`](JobRunOutcome::Running) while in flight).
    pub outcome: JobRunOutcome,
}

impl JobRunSummary {
    /// The run's wall-clock duration, or `None` if it has not finished.
    pub fn duration(&self) -> Option<Duration> {
        self.finished_at?.duration_since(self.started_at).ok()
    }
}

/// Optional descriptive metadata attached to a job, surfaced through [`JobInfo`].
#[derive(Debug, Clone, Default)]
pub struct JobMetadata {
    /// The job's stable name (`"Type::method"` for static jobs, caller-chosen for dynamic).
    pub name: Arc<str>,
    /// Arbitrary key/value labels, e.g. for grouping in a dashboard.
    pub labels: BTreeMap<String, String>,
    /// A human-readable description.
    pub description: Option<String>,
}

impl JobMetadata {
    /// Metadata carrying only a name (no labels or description).
    pub fn named(name: Arc<str>) -> Self {
        Self {
            name,
            labels: BTreeMap::new(),
            description: None,
        }
    }
}

/// A read-only snapshot of one job's state, returned by
/// [`list_jobs`](crate::JobScheduler::list_jobs) / [`job`](crate::JobScheduler::job).
#[derive(Debug, Clone)]
pub struct JobInfo {
    /// The job's identifier.
    pub id: JobId,
    /// The job's stable name.
    pub name: Arc<str>,
    /// A description of the job's schedule.
    pub schedule: ScheduleInfo,
    /// The job's current lifecycle state.
    pub state: JobState,
    /// The next scheduled run, if known and not paused.
    pub next_run_at: Option<SystemTime>,
    /// A summary of the most recent run, if any.
    pub last_run: Option<JobRunSummary>,
    /// The latest progress snapshot reported by the active run, if any.
    pub progress: Option<JobProgress>,
    /// How many runs have completed (successfully or not).
    pub run_count: u64,
    /// How many runs have failed or timed out.
    pub failure_count: u64,
    /// How many firings were skipped because a run was already active.
    pub skipped_count: u64,
    /// The job's labels.
    pub labels: BTreeMap<String, String>,
    /// The job's description.
    pub description: Option<String>,
}

/// The mutable runtime state of a job, guarded by one mutex.
struct Shared {
    state: JobState,
    schedule: Arc<Schedule>,
    options: JobOptions,
    next_run_at: Option<SystemTime>,
    run_count: u64,
    failure_count: u64,
    skipped_count: u64,
    last_run: Option<JobRunSummary>,
    recent: VecDeque<JobRunSummary>,
    paused: bool,
}

/// The shared record of one scheduled job.
///
/// One `Arc<JobEntry>` lives in the [`JobRegistry`] and is captured by the background loop
/// driving the job, so a control call (pause, reschedule, cancel) and the running loop see the
/// same state. Its mutable fields sit behind a single mutex ([`Shared`]) plus a few atomics /
/// notifies used for wake-ups and the overlap guard.
pub(crate) struct JobEntry {
    pub id: JobId,
    pub metadata: JobMetadata,
    /// Per-job cancel token (a child of the scheduler's), also used as the run token's parent.
    pub token: CancellationToken,
    /// The re-runnable job body.
    pub runner: Runner,
    /// The scheduler-shared run-id allocator.
    pub run_ids: Arc<AtomicU64>,
    shared: Mutex<Shared>,
    /// The latest progress snapshot of the active run, shared with its [`JobRunContext`].
    progress: Arc<Mutex<Option<JobProgress>>>,
    /// Notified on resume / reschedule so the loop recomputes its wait.
    wake: Notify,
    /// Notified when a manual run has been queued.
    triggered: Notify,
    /// Manual runs awaiting execution, with their pre-allocated run ids.
    manual: Mutex<VecDeque<JobRunId>>,
    /// In-flight run count, for the overlap guard.
    active: AtomicUsize,
    /// The firing deferred under [`OverlapPolicy::QueueOne`], carrying its pre-allocated run id
    /// and trigger so the eventual run keeps its identity (a manual defer stays manual).
    pending: Mutex<Option<(JobRunId, JobTrigger)>>,
    /// The active run's id and cancel token, for [`OverlapPolicy::CancelPrevious`]. Tagged with
    /// the owning run id so a late-finishing older run cannot clear a replacement's token.
    run_token: Mutex<Option<(JobRunId, CancellationToken)>>,
    /// Cancelled by the loop as it exits, so [`cancel_and_wait`](crate::JobScheduler::cancel_and_wait)
    /// can await teardown. Awaiting it after exit resolves immediately.
    done: CancellationToken,
}

impl JobEntry {
    /// Builds an entry in the [`Scheduled`](JobState::Scheduled) state.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: JobId,
        metadata: JobMetadata,
        token: CancellationToken,
        runner: Runner,
        run_ids: Arc<AtomicU64>,
        schedule: Arc<Schedule>,
        options: JobOptions,
    ) -> Self {
        Self {
            id,
            metadata,
            token,
            runner,
            run_ids,
            shared: Mutex::new(Shared {
                state: JobState::Scheduled,
                schedule,
                options,
                next_run_at: None,
                run_count: 0,
                failure_count: 0,
                skipped_count: 0,
                last_run: None,
                recent: VecDeque::new(),
                paused: false,
            }),
            progress: Arc::new(Mutex::new(None)),
            wake: Notify::new(),
            triggered: Notify::new(),
            manual: Mutex::new(VecDeque::new()),
            active: AtomicUsize::new(0),
            pending: Mutex::new(None),
            run_token: Mutex::new(None),
            done: CancellationToken::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().expect("job entry not poisoned")
    }

    /// A read-only snapshot of the job's current state.
    pub fn info(&self) -> JobInfo {
        let shared = self.lock();

        JobInfo {
            id: self.id,
            name: Arc::clone(&self.metadata.name),
            schedule: shared.schedule.describe(),
            state: shared.state.clone(),
            next_run_at: shared.next_run_at,
            last_run: shared.last_run.clone(),
            progress: self.progress.lock().expect("progress not poisoned").clone(),
            run_count: shared.run_count,
            failure_count: shared.failure_count,
            skipped_count: shared.skipped_count,
            labels: self.metadata.labels.clone(),
            description: self.metadata.description.clone(),
        }
    }

    /// The most recent runs, oldest first.
    pub fn recent_runs(&self) -> Vec<JobRunSummary> {
        self.lock().recent.iter().cloned().collect()
    }

    /// The current schedule (cheap `Arc` clone; the loop reads this each iteration so a
    /// reschedule is observed).
    pub fn schedule(&self) -> Arc<Schedule> {
        Arc::clone(&self.lock().schedule)
    }

    /// The current execution options.
    pub fn options(&self) -> JobOptions {
        self.lock().options.clone()
    }

    /// Whether scheduling is suspended.
    pub fn is_paused(&self) -> bool {
        self.lock().paused
    }

    /// The progress slot, shared into a run's [`JobRunContext`].
    pub fn progress_slot(&self) -> Arc<Mutex<Option<JobProgress>>> {
        Arc::clone(&self.progress)
    }

    /// Records the next expected run time (for introspection).
    pub fn set_next_run_at(&self, at: Option<SystemTime>) {
        self.lock().next_run_at = at;
    }

    /// Marks the job cancelling and cancels its token.
    pub fn begin_cancel(&self) {
        self.lock().state = JobState::Cancelling;
        self.token.cancel();
    }

    /// Marks the job cancelled (terminal) and signals loop teardown, called by the loop as it
    /// exits.
    pub fn mark_cancelled(&self) {
        self.lock().state = JobState::Cancelled;
        self.done.cancel();
    }

    /// Awaits the loop's teardown (resolves immediately if it has already ended).
    pub async fn wait_done(&self) {
        self.done.cancelled().await;
    }

    /// Pauses scheduling. Returns whether the state changed.
    pub fn pause(&self) -> bool {
        let mut shared = self.lock();

        if shared.paused {
            return false;
        }

        shared.paused = true;

        if matches!(shared.state, JobState::Scheduled) {
            shared.state = JobState::Paused;
        }

        drop(shared);
        self.wake.notify_one();

        true
    }

    /// Resumes scheduling and asks the loop to recompute its next fire. Returns whether the
    /// state changed.
    pub fn resume(&self) -> bool {
        let mut shared = self.lock();

        if !shared.paused {
            return false;
        }

        shared.paused = false;

        if matches!(shared.state, JobState::Paused) {
            shared.state = JobState::Scheduled;
        }

        drop(shared);
        self.wake.notify_one();

        true
    }

    /// Swaps the schedule and wakes the loop to recompute.
    pub fn reschedule(&self, schedule: Schedule) {
        self.lock().schedule = Arc::new(schedule);
        self.wake.notify_one();
    }

    /// Queues a manual run with a freshly allocated run id and wakes the loop.
    pub fn trigger_manual(&self) -> JobRunId {
        let run_id = self.next_run_id();

        self.manual
            .lock()
            .expect("manual queue not poisoned")
            .push_back(run_id);
        self.triggered.notify_one();

        run_id
    }

    /// Drains the queued manual run ids.
    pub fn take_manual(&self) -> Vec<JobRunId> {
        self.manual
            .lock()
            .expect("manual queue not poisoned")
            .drain(..)
            .collect()
    }

    /// Allocates the next run id from the scheduler-shared counter.
    pub fn next_run_id(&self) -> JobRunId {
        JobRunId(self.run_ids.fetch_add(1, Ordering::Relaxed))
    }

    /// Awaits the next wake (resume / reschedule / pause).
    pub async fn woken(&self) {
        self.wake.notified().await;
    }

    /// Awaits the next queued manual run notification.
    pub async fn triggered(&self) {
        self.triggered.notified().await;
    }

    // -- overlap guard ----------------------------------------------------

    /// The current in-flight run count.
    pub fn active(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }

    /// Marks a run started (increments the in-flight count).
    pub fn enter_run(&self) {
        self.active.fetch_add(1, Ordering::SeqCst);
    }

    /// Marks a run finished; returns the [`QueueOne`](crate::OverlapPolicy::QueueOne) firing
    /// deferred while it ran (and clears it), so the caller can start it with its original id
    /// and trigger.
    pub fn exit_run(&self) -> Option<(JobRunId, JobTrigger)> {
        self.active.fetch_sub(1, Ordering::SeqCst);

        self.pending.lock().expect("pending not poisoned").take()
    }

    /// Records a firing deferred under [`QueueOne`](crate::OverlapPolicy::QueueOne), preserving
    /// its run id and trigger. At most one is queued; a later firing while one is already
    /// pending is dropped.
    pub fn mark_pending(&self, run_id: JobRunId, trigger: JobTrigger) {
        let mut pending = self.pending.lock().expect("pending not poisoned");

        if pending.is_none() {
            *pending = Some((run_id, trigger));
        }
    }

    /// Records the active run's cancel token (for `CancelPrevious`), tagged with its run id.
    pub fn set_run_token(&self, run_id: JobRunId, token: CancellationToken) {
        *self.run_token.lock().expect("run token not poisoned") = Some((run_id, token));
    }

    /// Clears the active run token, but only if `run_id` still owns it — a late older run must
    /// not clear a replacement's token under `CancelPrevious`.
    pub fn clear_run_token(&self, run_id: JobRunId) {
        let mut slot = self.run_token.lock().expect("run token not poisoned");

        if slot.as_ref().is_some_and(|(owner, _)| *owner == run_id) {
            *slot = None;
        }
    }

    /// Cancels the active run's token, if any.
    pub fn cancel_active_run(&self) {
        if let Some((_, token)) = self
            .run_token
            .lock()
            .expect("run token not poisoned")
            .as_ref()
        {
            token.cancel();
        }
    }

    // -- run bookkeeping --------------------------------------------------

    /// Records a run start: sets state `Running`, resets progress, and pushes a
    /// still-running summary. Returns the pushed summary's index-agnostic run id.
    pub fn record_start(&self, run_id: JobRunId, trigger: JobTrigger, started_at: SystemTime) {
        *self.progress.lock().expect("progress not poisoned") = None;

        let mut shared = self.lock();

        if !matches!(shared.state, JobState::Cancelling | JobState::Cancelled) {
            shared.state = JobState::Running { run_id };
        }

        let summary = JobRunSummary {
            run_id,
            trigger,
            started_at,
            finished_at: None,
            outcome: JobRunOutcome::Running,
        };

        shared.last_run = Some(summary.clone());
        shared.recent.push_back(summary);

        while shared.recent.len() > RECENT_RUNS_CAP {
            shared.recent.pop_front();
        }
    }

    /// Records a firing skipped by the overlap guard.
    pub fn record_skipped(&self) {
        self.lock().skipped_count += 1;
    }

    /// Records a run finish: updates counters, the recent buffer, and the state (back to
    /// `Scheduled`/`Paused` once no runs remain active).
    pub fn record_finish(&self, run_id: JobRunId, finished_at: SystemTime, outcome: JobRunOutcome) {
        let mut shared = self.lock();

        shared.run_count += 1;

        if matches!(outcome, JobRunOutcome::Failed(_) | JobRunOutcome::TimedOut) {
            shared.failure_count += 1;
        }

        let finalize = |summary: &mut JobRunSummary| {
            if summary.run_id == run_id {
                summary.finished_at = Some(finished_at);
                summary.outcome = outcome.clone();
            }
        };

        if let Some(last) = shared.last_run.as_mut() {
            finalize(last);
        }

        for summary in shared.recent.iter_mut() {
            finalize(summary);
        }

        if !matches!(shared.state, JobState::Cancelling | JobState::Cancelled) && self.active() <= 1
        {
            shared.state = if shared.paused {
                JobState::Paused
            } else {
                JobState::Scheduled
            };
        }
    }
}

/// The registry of live jobs, keyed by [`JobId`].
///
/// Holds one `Arc<JobEntry>` per job; the scheduler inserts on spawn and removes when a loop
/// ends. Lookups by id and by name back the public introspection and control APIs.
#[derive(Default)]
pub(crate) struct JobRegistry {
    entries: Mutex<HashMap<JobId, Arc<JobEntry>>>,
}

impl JobRegistry {
    /// Inserts an entry.
    pub fn insert(&self, entry: Arc<JobEntry>) {
        self.entries
            .lock()
            .expect("registry not poisoned")
            .insert(entry.id, entry);
    }

    /// Removes the entry for `id`.
    pub fn remove(&self, id: JobId) {
        self.entries
            .lock()
            .expect("registry not poisoned")
            .remove(&id);
    }

    /// The entry for `id`, if present.
    pub fn get(&self, id: JobId) -> Option<Arc<JobEntry>> {
        self.entries
            .lock()
            .expect("registry not poisoned")
            .get(&id)
            .cloned()
    }

    /// The first entry whose name matches `name`.
    pub fn by_name(&self, name: &str) -> Option<Arc<JobEntry>> {
        self.entries
            .lock()
            .expect("registry not poisoned")
            .values()
            .find(|entry| &*entry.metadata.name == name)
            .cloned()
    }

    /// Every live entry.
    pub fn entries(&self) -> Vec<Arc<JobEntry>> {
        self.entries
            .lock()
            .expect("registry not poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// Whether the registry is empty (used in tests).
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries
            .lock()
            .expect("registry not poisoned")
            .is_empty()
    }
}
