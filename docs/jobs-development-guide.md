# Jobs Development Guide

This document captures the next development direction for `overseerd-jobs`.
The current implementation provides the core scheduling path: static `#[job]`
discovery, dynamic scheduling, interval and cron schedules, per-run DI,
cancellation, and tracing logs. The next useful step is to make jobs observable
and controllable while keeping storage and operational policy pluggable.

## Goals

- Give applications a first-class way to inspect scheduled jobs and recent runs.
- Support per-run progress and logs without coupling `overseerd-jobs` to a
  specific storage backend.
- Add operational controls such as trigger-now, pause/resume, and reschedule.
- Preserve the lightweight scheduler model: no persistence requirement, no
  bundled database, and no protocol dependency.

## 1. Job Status and Introspection

`JobScheduler` should expose a read API for scheduled jobs and recent runs.
Today it stores only `JobId -> CancellationToken`, which is enough for
cancellation but not enough for observability.

Proposed concepts:

```rust
pub struct JobInfo {
    pub id: JobId,
    pub name: Arc<str>,
    pub schedule: ScheduleInfo,
    pub state: JobState,
    pub next_run_at: Option<SystemTime>,
    pub last_run: Option<JobRunSummary>,
    pub run_count: u64,
    pub failure_count: u64,
}

pub enum JobState {
    Scheduled,
    Running { run_id: JobRunId },
    Paused,
    Cancelling,
    Cancelled,
}
```

Useful APIs:

```rust
impl JobScheduler {
    pub fn list_jobs(&self) -> Vec<JobInfo>;
    pub fn job(&self, id: JobId) -> Option<JobInfo>;
    pub fn recent_runs(&self, id: JobId) -> Vec<JobRunSummary>;
}
```

The scheduler should track static and dynamic jobs through the same registry so
introspection does not care where a job came from.

## 2. Progress Reporting

Jobs should be able to report run progress while they execute. This is separate
from logs: progress is structured state, while logs are event streams.

Proposed approach:

- Create a `JobRunContext` per run.
- Allow job methods to request it as an injected parameter.
- Store the latest progress snapshot through the same observability backend used
  for run metadata.

Example shape:

```rust
pub struct JobRunContext {
    pub run_id: JobRunId,
    pub job_id: JobId,
}

impl JobRunContext {
    pub async fn progress(&self, progress: JobProgress);
}

pub struct JobProgress {
    pub phase: Option<String>,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub message: Option<String>,
}
```

The macro path should support this without a special wrapper if possible:

```rust
#[job(every = "5m")]
async fn rebuild_index(&self, cx: JobRunContext) -> Result<()> {
    cx.progress(JobProgress::phase("loading")).await;
    Ok(())
}
```

## 4. Manual Run / Trigger Now

Operators and tests need a way to run a job immediately without waiting for the
next scheduled occurrence.

Proposed APIs:

```rust
impl JobScheduler {
    pub async fn run_now(&self, id: JobId) -> Result<JobRunId, JobError>;
    pub async fn run_named(&self, name: &str) -> Result<JobRunId, JobError>;
}
```

Important behavior:

- Respect the job's overlap policy.
- Record the manual run exactly like a scheduled run.
- Mark the trigger source in run metadata.

```rust
pub enum JobTrigger {
    Schedule,
    Manual,
}
```

## 5. Pause, Resume, and Reschedule

`JobHandle` currently supports cancellation only. Production use needs softer
control.

Proposed APIs:

```rust
impl JobScheduler {
    pub fn pause(&self, id: JobId) -> Result<(), JobError>;
    pub fn resume(&self, id: JobId) -> Result<(), JobError>;
    pub fn reschedule(&self, id: JobId, schedule: Schedule) -> Result<(), JobError>;
    pub async fn cancel_and_wait(&self, id: JobId) -> Result<(), JobError>;
}
```

Expected semantics:

- `pause` prevents future scheduled runs but does not abort the currently active
  run.
- `resume` re-enables scheduling and recomputes the next fire time.
- `reschedule` changes the future cadence without losing run history.
- `cancel_and_wait` is useful for graceful shutdown and tests.

## 6. Named Dynamic Jobs

Dynamic jobs are currently logged as `"dynamic"`, which is not enough once an
application schedules more than one runtime job.

Proposed API:

```rust
impl JobScheduler {
    pub fn schedule_named<F, Fut>(
        &self,
        name: impl Into<Arc<str>>,
        schedule: Schedule,
        run: F,
    ) -> JobHandle
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = JobOutcome> + Send + 'static;
}
```

The existing `schedule` API can remain as a convenience wrapper that assigns a
generated name or a `"dynamic"` fallback.

Optional metadata:

```rust
pub struct JobMetadata {
    pub name: Arc<str>,
    pub labels: BTreeMap<String, String>,
    pub description: Option<String>,
}
```

## 7. Metrics and Health Hooks

The scheduler should emit structured operational data independent of any
specific metrics backend.

Useful measurements:

- jobs scheduled
- active runs
- completed runs
- failed runs
- cancelled runs
- skipped ticks
- run duration
- schedule lag
- stale jobs

The runtime should expose these through a small observation interface and let
the application connect it to `metrics`, OpenTelemetry, health endpoints, or its
own system.

Health should be policy-driven. `overseerd-jobs` should provide enough state for
an application to decide whether a failed or stale job makes the process
unhealthy; it should not hard-code that policy.

## 8. Schedule Options

The schedule model should grow from `Every(Duration)` and `Cron(Cron)` into a
schedule plus execution policy.

Proposed options:

```rust
pub struct JobOptions {
    pub run_on_startup: bool,
    pub timeout: Option<Duration>,
    pub jitter: Option<Duration>,
    pub overlap: OverlapPolicy,
    pub max_runtime: Option<Duration>,
    pub timezone: Option<JobTimezone>,
}

pub enum OverlapPolicy {
    Skip,
    QueueOne,
    Allow,
    CancelPrevious,
}
```

Initial default should preserve current behavior:

- interval jobs do not run immediately
- missed interval ticks are skipped
- runs do not overlap
- cron is computed from wall-clock UTC

## Per-Run Log Capture

Each job run should execute inside a dedicated tracing span. That gives the
observability layer a stable correlation key without requiring a per-run global
subscriber.

Proposed run span:

```rust
let span = tracing::info_span!(
    target: "overseerd::jobs",
    "job.run",
    job_id = %job_id,
    job_name = %name,
    run_id = %run_id,
);

let outcome = run().instrument(span).await;
```

Logs emitted by the job and awaited child futures will carry the run span.
Tasks spawned with `tokio::spawn` need explicit instrumentation if they should
remain associated with the run.

## Generic Log Storage

Log storage should be configurable and generic. The default can be bounded
in-memory storage for local observability, but applications must be able to
store logs out of process.

Proposed trait:

```rust
#[async_trait]
pub trait JobLogSink: Send + Sync + 'static {
    async fn record(&self, event: JobLogRecord);
    async fn records(&self, run_id: JobRunId, limit: usize) -> Result<Vec<JobLogRecord>, JobError>;
}
```

Potential implementations:

- `InMemoryJobLogStore`: bounded by max runs, max bytes per run, and TTL.
- `NoopJobLogStore`: default when capture is disabled.
- Application-provided sinks for databases, object storage, log pipelines, or
  remote observability services.

Configuration should control:

```rust
pub struct JobLogConfig {
    pub enabled: bool,
    pub max_runs: usize,
    pub max_bytes_per_run: usize,
    pub ttl: Duration,
    pub capture_level: tracing::Level,
}
```

The tracing integration should be a normal `tracing_subscriber::Layer` that
captures events whose span stack contains a `job.run` span with a `run_id`.
This keeps capture compatible with the application's existing tracing setup.

## Suggested Implementation Order

1. Replace the scheduler's internal registry with a `JobRegistry` that stores
   metadata, state, cancellation tokens, and recent run summaries.
2. Add named dynamic jobs and public `list_jobs` / `job` APIs.
3. Introduce `JobRunId`, per-run spans, and run summaries.
4. Add the generic `JobLogSink` abstraction with a bounded in-memory
   implementation.
5. Add manual `run_now`.
6. Add pause/resume/reschedule.
7. Add `JobRunContext` progress reporting.
8. Add metrics/health observation hooks.
9. Expand schedule options while preserving current defaults.
