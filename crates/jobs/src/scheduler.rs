//! The `JobScheduler` singleton: spawns one supervised loop per job.
//!
//! Jobs come from two places, both funnelling through the same [`spawn`](JobScheduler::spawn)
//! path:
//!
//! - **static** — every `#[job]` in the binary, discovered at link time from the [`JOBS`]
//!   slice and started by the scheduler's `Startup` hook.
//! - **dynamic** — added at run time through [`JobScheduler::schedule`], for jobs whose
//!   existence or cadence is only known then (e.g. loaded from a database). The scheduler is
//!   an injectable singleton, so any component can take `Arc<JobScheduler>` and schedule work;
//!   the returned [`JobHandle`] cancels (un-registers) that job.
//!
//! The scheduler is a framework-internal component. Like the other framework singletons
//! (`ShutdownHandle`, `HookManager`, `ConfigReloader`), it hand-rolls its DI descriptors
//! rather than going through the `#[service]`/`#[hook]` macros — which root their generated
//! paths at the `overseerd` facade and so cannot be used from a crate *below* it.
//!
//! Each job runs on its own [child token](CancellationToken::child_token) of the scheduler's
//! token, so cancelling one job stops only that job while dropping the scheduler (on shutdown)
//! cancels every one. Because a run is `await`ed sequentially inside its loop, a job whose
//! body outlasts its period simply skips the ticks it missed — the built-in overlap guard.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use croner::Cron;
use overseerd_core::{DependencyDescriptor, ResolverCtx, ResolverCtxExt, TypeDescriptor};
use overseerd_di::{
    BoxedComponent, Component, ComponentConstructionContext, ComponentDescriptor,
    ComponentFactoryDescriptor, ComponentSource, Result as DiResult, RootResolver, Singleton,
    dispatch_factory, factory_dependencies,
};
use overseerd_hooks::{
    Error as HookError, HookDescriptor, HookKind, Result as HookResult, Startup,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, trace};

use crate::descriptor::{JOBS, JobOutcome};
use crate::schedule::{Schedule, ScheduleError};

#[cfg(test)]
mod tests;

/// The stable component id of the [`JobScheduler`] singleton.
const SCHEDULER_ID: &str = "overseerd:job-scheduler";

/// The display name of the [`JobScheduler`] singleton.
const SCHEDULER_NAME: &str = "JobScheduler";

/// An erased, re-runnable job body: called once per tick, producing a [`JobOutcome`].
///
/// Static `#[job]`s wrap their generated call (closing over a cloned [`RootResolver`]);
/// dynamic jobs wrap the user's closure. `Arc` + `Fn` so one runner drives every tick.
type Runner =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = JobOutcome> + Send + 'static>> + Send + Sync>;

/// An opaque identifier for a scheduled job, unique within one scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(u64);

/// A handle to one scheduled job, returned by [`JobScheduler::schedule`].
///
/// Use it to [`cancel`](Self::cancel) (un-register) the job. Cloning shares the same
/// underlying job; cancelling through any clone stops it. Dropping the handle does **not**
/// cancel the job — a scheduled job runs until explicitly cancelled or the scheduler is
/// dropped — so a fire-and-forget caller may discard it.
#[derive(Clone)]
pub struct JobHandle {
    id: JobId,
    token: CancellationToken,
}

impl JobHandle {
    /// This job's identifier.
    pub fn id(&self) -> JobId {
        self.id
    }

    /// Cancels (un-registers) the job: its loop stops at the next tick or wake, and it is
    /// removed from the scheduler. Idempotent.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// Whether the job has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }
}

/// Drives every job on its schedule.
///
/// Seeded as an injectable singleton by the [`JobsPlugin`](crate::plugin::JobsPlugin): its
/// `Startup` hook starts the static `#[job]`s, and any component can inject
/// `Arc<JobScheduler>` to [`schedule`](Self::schedule) more at run time.
pub struct JobScheduler {
    root: RootResolver,
    /// Parent token: cancelling it (on `Drop`) stops every job. Each job holds a child.
    cancel: CancellationToken,
    next_id: AtomicU64,
    /// Live jobs by id, for individual cancellation and self-cleanup when a loop ends.
    jobs: Arc<Mutex<HashMap<JobId, CancellationToken>>>,
}

impl JobScheduler {
    /// The factory body: injects the framework-seeded [`RootResolver`].
    async fn create(root: RootResolver) -> Self {
        Self {
            root,
            cancel: CancellationToken::new(),
            next_id: AtomicU64::new(0),
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Schedules a job to run on `schedule`, invoking `run` on each occurrence. Returns a
    /// [`JobHandle`] to cancel it. The closure is run fresh each tick, so it may capture live
    /// dependencies (a `Dep<T>`, or an injected [`RootResolver`] to resolve per run).
    ///
    /// Runs immediately — no startup phase — so it is the right entry point for jobs
    /// discovered at run time (e.g. loaded from a database).
    pub fn schedule<F, Fut>(&self, schedule: Schedule, run: F) -> JobHandle
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = JobOutcome> + Send + 'static,
    {
        let run = Arc::new(run);
        let runner: Runner = Arc::new(move || {
            let run = Arc::clone(&run);

            Box::pin(async move { run().await })
        });

        self.spawn("dynamic".into(), schedule, runner)
    }

    /// Parses every static `#[job]`'s schedule and spawns its loop. An unparseable schedule
    /// aborts startup, so a misconfigured `#[job]` fails loudly rather than never running.
    async fn start(&self) -> Result<(), ScheduleError> {
        let mut count = 0;

        for job in JOBS.iter() {
            let schedule = Schedule::parse(job.kind, job.schedule)?;
            let root = self.root.clone();
            let call = job.call;
            let runner: Runner = Arc::new(move || call(root.clone()));

            self.spawn(job.name.into(), schedule, runner);

            count += 1;

            info!(
                target: "overseerd::jobs",
                job = job.name,
                schedule = job.schedule,
                "job scheduled"
            );
        }

        info!(target: "overseerd::jobs", count, "job scheduler started");

        Ok(())
    }

    /// The shared spawn path: registers a child token, launches the loop for `schedule`, and
    /// arranges the job to remove itself from the registry when its loop ends.
    fn spawn(&self, name: Arc<str>, schedule: Schedule, run: Runner) -> JobHandle {
        let id = JobId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let token = self.cancel.child_token();

        self.jobs
            .lock()
            .expect("scheduler registry not poisoned")
            .insert(id, token.clone());

        let registry = Arc::clone(&self.jobs);
        let loop_token = token.clone();

        tokio::spawn(async move {
            match schedule {
                Schedule::Every(period) => run_interval(&name, &run, &loop_token, period).await,
                Schedule::Cron(cron) => run_cron(&name, &run, &loop_token, *cron).await,
            }

            registry
                .lock()
                .expect("scheduler registry not poisoned")
                .remove(&id);
        });

        JobHandle { id, token }
    }
}

/// Cancels every loop when the scheduler is torn down (the root container drops on shutdown).
impl Drop for JobScheduler {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Runs a job every `period`, on the monotonic timer. The immediate first tick is consumed so
/// `every = "30s"` first fires after 30s rather than at startup; missed ticks are skipped,
/// which is what makes a long-running body defer (not queue) its next run.
async fn run_interval(name: &str, run: &Runner, cancel: &CancellationToken, period: Duration) {
    // `tokio::time::interval` panics on a zero period. `Schedule::parse` rejects it, but the
    // public `Schedule::Every` variant can still carry one, so guard here rather than panic the
    // task: a zero-period job never runs and is logged.
    if period.is_zero() {
        error!(
            target: "overseerd::jobs",
            job = name,
            "interval job has a zero period; it will not run"
        );

        return;
    }

    let mut ticker = tokio::time::interval(period);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    ticker.tick().await;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = ticker.tick() => run_once(name, run).await,
        }
    }
}

/// Runs a job at each occurrence of `cron`, computing the next fire time from the wall clock
/// after each run (so a slow run defers rather than queues its next occurrence).
async fn run_cron(name: &str, run: &Runner, cancel: &CancellationToken, cron: Cron) {
    loop {
        let now = Utc::now();

        let next = match cron.find_next_occurrence(&now, false) {
            Ok(next) => next,

            Err(error) => {
                error!(
                    target: "overseerd::jobs",
                    job = name,
                    %error,
                    "no next cron occurrence; stopping this job"
                );

                break;
            }
        };

        let wait = (next - now).to_std().unwrap_or(Duration::ZERO);

        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(wait) => run_once(name, run).await,
        }
    }
}

/// Invokes a job's runner once, logging the outcome. Errors are logged and swallowed — a job
/// failure never tears the scheduler (or the daemon) down.
async fn run_once(name: &str, run: &Runner) {
    match run().await {
        Ok(()) => trace!(target: "overseerd::jobs", job = name, "job run completed"),

        Err(error) => error!(
            target: "overseerd::jobs",
            job = name,
            %error,
            "job run failed"
        ),
    }
}

// ---------------------------------------------------------------------------
// Hand-rolled DI descriptors (the `#[service]`/`#[hook]` macros cannot be used below the
// facade). This is the runtime metadata a `#[service] #[methods]` pair would otherwise emit.
// ---------------------------------------------------------------------------

impl Component for JobScheduler {
    const ID: &'static str = SCHEDULER_ID;
    const NAME: &'static str = SCHEDULER_NAME;
    type Handle = Arc<JobScheduler>;

    fn into_handle(self) -> Arc<JobScheduler> {
        Arc::new(self)
    }
}

/// Under `di-check`, the scheduler is plugin-seeded, so it is always provided — letting a user
/// component inject `Arc<JobScheduler>` to schedule jobs at run time.
#[cfg(feature = "di-check")]
impl overseerd_di::Provide<JobScheduler> for overseerd_di::Wiring {}

/// The scheduler's construction factory: resolves the injected [`RootResolver`] and builds it.
fn scheduler_construct<'a>(
    cx: &'a mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = DiResult<BoxedComponent>> + Send + 'a>> {
    dispatch_factory(JobScheduler::create, cx)
}

/// The scheduler's dependency edges, recovered from its factory's parameters.
fn scheduler_deps() -> Vec<DependencyDescriptor> {
    factory_dependencies(JobScheduler::create)
}

static SCHEDULER_FACTORIES: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
    construct: scheduler_construct,
    dependencies: scheduler_deps,
    default: false,
}];

fn scheduler_factories() -> &'static [ComponentFactoryDescriptor] {
    &SCHEDULER_FACTORIES
}

/// The boxed future a hook call returns — the same shape `overseerd-hooks` uses internally,
/// named here so the hand-rolled hook signature stays readable.
type HookCallFuture<'a> =
    Pin<Box<dyn Future<Output = HookResult<Box<dyn Any + Send>>> + Send + 'a>>;

/// The erased `Startup` hook: resolves the scheduler receiver through the component source and
/// starts its static jobs. Mirrors the body the `#[hook(Startup)]` macro would generate.
fn scheduler_startup_call<'a>(
    ctx: &'a (dyn ResolverCtx + Send + Sync),
    _cx: &'a (dyn Any + Send + Sync),
) -> HookCallFuture<'a> {
    Box::pin(async move {
        let scheduler = ctx
            .get_resolver::<ComponentSource>()
            .and_then(|source| source.component::<JobScheduler>())
            .ok_or(HookError::MissingReceiver(SCHEDULER_NAME))?;

        scheduler
            .start()
            .await
            .map_err(|error| HookError::Other(Box::new(error)))?;

        Ok(Box::new(()) as Box<dyn Any + Send>)
    })
}

fn scheduler_startup_kind_ty() -> TypeId {
    TypeId::of::<Startup>()
}

fn scheduler_startup_deps() -> Vec<DependencyDescriptor> {
    Vec::new()
}

static SCHEDULER_HOOKS: [HookDescriptor; 1] = [HookDescriptor {
    component_ty: TypeDescriptor::of::<JobScheduler>(SCHEDULER_NAME),
    kind: <Startup as HookKind>::NAME,
    kind_ty: scheduler_startup_kind_ty,
    dependencies: scheduler_startup_deps,
    call: scheduler_startup_call,
}];

fn scheduler_hooks() -> &'static [HookDescriptor] {
    &SCHEDULER_HOOKS
}

/// The [`ComponentDescriptor`] for the scheduler singleton, registered by the
/// [`JobsPlugin`](crate::plugin::JobsPlugin).
pub(crate) fn scheduler_descriptor() -> ComponentDescriptor {
    ComponentDescriptor {
        id: SCHEDULER_ID,
        name: SCHEDULER_NAME,
        ty: TypeDescriptor::of::<JobScheduler>(SCHEDULER_NAME),
        scope: &Singleton,
        factories: scheduler_factories,
        hooks: scheduler_hooks,
    }
}
