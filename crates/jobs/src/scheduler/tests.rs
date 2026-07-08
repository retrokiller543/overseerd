use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use overseerd_di::{Component, RootResolver};
use overseerd_hooks::{HookKind, Startup};

use super::{JobScheduler, scheduler_descriptor};
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
    let runs = Arc::new(AtomicUsize::new(0));

    let counter = Arc::clone(&runs);
    let handle = scheduler.schedule(Schedule::every(Duration::from_millis(10)), move || {
        let counter = Arc::clone(&counter);

        async move {
            counter.fetch_add(1, Ordering::Relaxed);

            Ok(())
        }
    });

    // The first interval tick is consumed, so the first run lands after one period.
    tokio::time::sleep(Duration::from_millis(35)).await;
    let while_running = runs.load(Ordering::Relaxed);

    assert!(
        while_running >= 2,
        "expected repeated runs, got {while_running}"
    );

    handle.cancel();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let after_cancel = runs.load(Ordering::Relaxed);

    // No further runs happen once cancelled.
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(
        runs.load(Ordering::Relaxed),
        after_cancel,
        "ran after cancel"
    );
    assert!(
        scheduler.jobs.lock().unwrap().is_empty(),
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
