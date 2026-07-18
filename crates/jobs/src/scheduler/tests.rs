use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use overseerd_di::{Component, RootResolver};
use overseerd_hooks::{HookKind, Startup};

use super::{JobScheduler, scheduler_descriptor};
use crate::registry::{JobState, JobTrigger};
use crate::schedule::Schedule;

#[test]
fn descriptor_identity_matches_component() {
    let descriptor = scheduler_descriptor();

    assert_eq!(descriptor.id, <JobScheduler as Component>::ID);
    assert_eq!(descriptor.name, <JobScheduler as Component>::NAME);
    assert_eq!(descriptor.scope.name(), "Singleton");
}

#[test]
fn descriptor_carries_one_startup_hook() {
    let descriptor = scheduler_descriptor();
    let hooks = (descriptor.hooks)();

    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].kind, <Startup as HookKind>::NAME);
}

#[test]
fn descriptor_carries_a_non_default_factory() {
    let descriptor = scheduler_descriptor();
    let factories = (descriptor.factories)();

    assert_eq!(factories.len(), 1);
    assert!(!factories[0].default);
}

/// The scheduler needs no live container for dynamic jobs — the runner closure captures its
/// own state — so an unattached `RootResolver` is enough to exercise the runtime path.
#[tokio::test]
async fn dynamic_job_runs_then_cancels() {
    let scheduler = JobScheduler::create(RootResolver::new()).await;
    let (run_tx, mut run_rx) = tokio::sync::mpsc::unbounded_channel();

    let handle = scheduler.schedule(Schedule::every(Duration::from_millis(10)), move || {
        let run_tx = run_tx.clone();

        async move {
            run_tx.send(()).expect("test is still observing runs");

            Ok(())
        }
    });

    // Wait for observed runs instead of assuming a loaded runner schedules two ticks within a
    // narrow wall-clock window. The timeout is only a deadlock guard.
    for expected in 1..=2 {
        tokio::time::timeout(Duration::from_secs(1), run_rx.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for run {expected}"))
            .expect("job runner channel stays open");
    }

    scheduler
        .cancel_and_wait(handle.id())
        .await
        .expect("job exists");
    assert!(
        scheduler.registry.is_empty(),
        "cancelled job was not removed from the registry"
    );
}

#[tokio::test]
async fn dropping_scheduler_cancels_all_jobs() {
    let scheduler = JobScheduler::create(RootResolver::new()).await;
    let runs = Arc::new(AtomicUsize::new(0));

    let counter = Arc::clone(&runs);
    let _handle = scheduler.schedule(Schedule::every(Duration::from_millis(10)), move || {
        let counter = Arc::clone(&counter);

        async move {
            counter.fetch_add(1, Ordering::Relaxed);

            Ok(())
        }
    });

    tokio::time::sleep(Duration::from_millis(35)).await;
    drop(scheduler);

    let after_drop = runs.load(Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(
        runs.load(Ordering::Relaxed),
        after_drop,
        "ran after scheduler dropped"
    );
}

/// A counting dynamic job on a long interval, so only explicit triggers run it during a test.
fn counting_job(scheduler: &JobScheduler, name: &str, runs: Arc<AtomicUsize>) -> super::JobHandle {
    scheduler.schedule_named(
        name,
        Schedule::every(Duration::from_secs(3600)),
        move || {
            let runs = Arc::clone(&runs);

            async move {
                runs.fetch_add(1, Ordering::Relaxed);

                Ok(())
            }
        },
    )
}

#[tokio::test]
async fn run_now_triggers_a_manual_run() {
    let scheduler = JobScheduler::create(RootResolver::new()).await;
    let runs = Arc::new(AtomicUsize::new(0));
    let handle = counting_job(&scheduler, "Manual::job", Arc::clone(&runs));

    let run_id = scheduler.run_now(handle.id()).await.expect("job exists");
    tokio::time::sleep(Duration::from_millis(40)).await;

    assert_eq!(runs.load(Ordering::Relaxed), 1, "manual run did not fire");

    let recent = scheduler.recent_runs(handle.id());

    assert!(
        recent
            .iter()
            .any(|r| r.run_id == run_id && r.trigger == JobTrigger::Manual)
    );
}

#[tokio::test]
async fn run_now_on_unknown_job_errors() {
    let scheduler = JobScheduler::create(RootResolver::new()).await;
    let handle = counting_job(&scheduler, "Known::job", Arc::new(AtomicUsize::new(0)));
    handle.cancel();
    scheduler
        .cancel_and_wait(handle.id())
        .await
        .expect("job exists");

    assert!(scheduler.run_now(handle.id()).await.is_err());
}

#[tokio::test]
async fn list_jobs_and_job_reflect_registration() {
    let scheduler = JobScheduler::create(RootResolver::new()).await;
    let handle = counting_job(&scheduler, "Listed::job", Arc::new(AtomicUsize::new(0)));

    assert_eq!(scheduler.list_jobs().len(), 1);

    let info = scheduler.job(handle.id()).expect("job listed");

    assert_eq!(&*info.name, "Listed::job");
    assert_eq!(info.state, JobState::Scheduled);
}

#[tokio::test]
async fn pause_prevents_scheduled_runs() {
    let scheduler = JobScheduler::create(RootResolver::new()).await;
    let runs = Arc::new(AtomicUsize::new(0));
    let handle =
        scheduler.schedule_named("Paused::job", Schedule::every(Duration::from_millis(15)), {
            let runs = Arc::clone(&runs);

            move || {
                let runs = Arc::clone(&runs);

                async move {
                    runs.fetch_add(1, Ordering::Relaxed);

                    Ok(())
                }
            }
        });

    tokio::time::sleep(Duration::from_millis(40)).await;
    scheduler.pause(handle.id()).expect("job exists");

    let at_pause = runs.load(Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(60)).await;

    assert_eq!(runs.load(Ordering::Relaxed), at_pause, "ran while paused");
    assert_eq!(scheduler.job(handle.id()).unwrap().state, JobState::Paused);

    scheduler.resume(handle.id()).expect("job exists");
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(runs.load(Ordering::Relaxed) > at_pause, "did not resume");
}

#[tokio::test]
async fn cancel_and_wait_removes_the_job() {
    let scheduler = JobScheduler::create(RootResolver::new()).await;
    let handle = counting_job(&scheduler, "Doomed::job", Arc::new(AtomicUsize::new(0)));

    scheduler
        .cancel_and_wait(handle.id())
        .await
        .expect("job exists");

    assert!(scheduler.job(handle.id()).is_none());
    assert!(scheduler.registry.is_empty());
}

#[tokio::test]
async fn reschedule_changes_the_reported_cadence() {
    use crate::schedule::ScheduleInfo;

    let scheduler = JobScheduler::create(RootResolver::new()).await;
    let handle = counting_job(&scheduler, "Recadenced::job", Arc::new(AtomicUsize::new(0)));

    scheduler
        .reschedule(handle.id(), Schedule::cron("@hourly").unwrap())
        .expect("job exists");

    // The reschedule wakes the loop; give it a moment to store the new schedule.
    tokio::time::sleep(Duration::from_millis(20)).await;

    let info = scheduler.job(handle.id()).expect("job exists");

    assert!(matches!(info.schedule, ScheduleInfo::Cron(_)));
}

#[tokio::test]
async fn timeout_marks_a_run_timed_out() {
    use crate::registry::JobRunOutcome;
    use crate::schedule::JobOptions;

    let scheduler = JobScheduler::create(RootResolver::new()).await;
    let options = JobOptions {
        timeout: Some(Duration::from_millis(20)),
        ..JobOptions::default()
    };
    let handle = scheduler.schedule_with(
        crate::registry::JobMetadata::named("Slow::job".into()),
        Schedule::every(Duration::from_secs(3600)),
        options,
        || async {
            tokio::time::sleep(Duration::from_millis(500)).await;

            Ok(())
        },
    );

    scheduler.run_now(handle.id()).await.expect("job exists");
    tokio::time::sleep(Duration::from_millis(80)).await;

    let info = scheduler.job(handle.id()).expect("job exists");

    assert_eq!(info.last_run.unwrap().outcome, JobRunOutcome::TimedOut);
    assert_eq!(info.failure_count, 1);
}

#[tokio::test]
async fn skip_overlap_defers_while_a_run_is_active() {
    let scheduler = JobScheduler::create(RootResolver::new()).await;
    // Default overlap is Skip; a body far longer than the period forces overlap.
    let handle = scheduler.schedule_named(
        "Skippy::job",
        Schedule::every(Duration::from_millis(15)),
        || async {
            tokio::time::sleep(Duration::from_millis(120)).await;

            Ok(())
        },
    );

    tokio::time::sleep(Duration::from_millis(90)).await;

    let info = scheduler.job(handle.id()).expect("job exists");

    assert!(info.skipped_count > 0, "expected firings to be skipped");

    handle.cancel();
}

#[tokio::test]
async fn queue_one_preserves_the_deferred_manual_run_identity() {
    use crate::schedule::{JobOptions, OverlapPolicy};

    let scheduler = JobScheduler::create(RootResolver::new()).await;
    let options = JobOptions {
        overlap: OverlapPolicy::QueueOne,
        ..JobOptions::default()
    };
    // Long interval + slow body: manual triggers drive the runs, and the second overlaps the
    // first so it is deferred under QueueOne.
    let handle = scheduler.schedule_with(
        crate::registry::JobMetadata::named("Queued::job".into()),
        Schedule::every(Duration::from_secs(3600)),
        options,
        || async {
            tokio::time::sleep(Duration::from_millis(80)).await;

            Ok(())
        },
    );

    let first = scheduler.run_now(handle.id()).await.expect("job exists");
    tokio::time::sleep(Duration::from_millis(20)).await;
    let second = scheduler.run_now(handle.id()).await.expect("job exists");

    // Let the first run finish and the deferred second run start and finish.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let recent = scheduler.recent_runs(handle.id());

    // The deferred run keeps the id `run_now` returned and stays classified as manual — it is
    // not restarted with a fresh id or reclassified as scheduled.
    let deferred = recent
        .iter()
        .find(|r| r.run_id == second)
        .expect("deferred manual run recorded under its returned id");

    assert_eq!(deferred.trigger, JobTrigger::Manual);
    assert!(recent.iter().any(|r| r.run_id == first));
}

#[tokio::test]
async fn allow_overlap_permits_concurrent_runs() {
    use crate::schedule::{JobOptions, OverlapPolicy};

    let scheduler = JobScheduler::create(RootResolver::new()).await;
    let options = JobOptions {
        overlap: OverlapPolicy::Allow,
        ..JobOptions::default()
    };
    let handle = scheduler.schedule_with(
        crate::registry::JobMetadata::named("Overlapping::job".into()),
        Schedule::every(Duration::from_millis(20)),
        options,
        || async {
            tokio::time::sleep(Duration::from_millis(100)).await;

            Ok(())
        },
    );

    tokio::time::sleep(Duration::from_millis(90)).await;

    assert!(
        scheduler.metrics().active_runs > 1,
        "expected overlapping runs under OverlapPolicy::Allow"
    );

    handle.cancel();
}
