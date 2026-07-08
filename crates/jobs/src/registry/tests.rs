use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, SystemTime};

use tokio_util::sync::CancellationToken;

use super::{JobEntry, JobId, JobMetadata, JobRegistry, JobRunOutcome, JobState, JobTrigger};
use crate::descriptor::JobOutcome;
use crate::run::{JobRunContext, Runner};
use crate::schedule::{JobOptions, Schedule};

fn noop_runner() -> Runner {
    Arc::new(|_cx: JobRunContext| Box::pin(async { Ok::<(), _>(()) as JobOutcome }))
}

fn entry(name: &str) -> Arc<JobEntry> {
    Arc::new(JobEntry::new(
        JobId::from_raw(0),
        JobMetadata::named(Arc::from(name)),
        CancellationToken::new(),
        noop_runner(),
        Arc::new(AtomicU64::new(0)),
        Arc::new(Schedule::every(Duration::from_secs(60))),
        JobOptions::default(),
    ))
}

#[test]
fn new_entry_starts_scheduled() {
    let entry = entry("Test::job");
    let info = entry.info();

    assert_eq!(info.state, JobState::Scheduled);
    assert_eq!(info.run_count, 0);
    assert_eq!(info.failure_count, 0);
    assert!(info.last_run.is_none());
}

#[test]
fn record_start_then_finish_updates_counts_and_state() {
    let entry = entry("Test::job");
    let run_id = entry.next_run_id();
    let now = SystemTime::now();

    entry.record_start(run_id, JobTrigger::Manual, now);

    assert_eq!(entry.info().state, JobState::Running { run_id });
    assert_eq!(
        entry.info().last_run.unwrap().outcome,
        JobRunOutcome::Running
    );

    entry.record_finish(
        run_id,
        now + Duration::from_millis(5),
        JobRunOutcome::Success,
    );

    let info = entry.info();

    assert_eq!(info.state, JobState::Scheduled);
    assert_eq!(info.run_count, 1);
    assert_eq!(info.failure_count, 0);
    assert_eq!(info.last_run.unwrap().outcome, JobRunOutcome::Success);
}

#[test]
fn failed_runs_bump_the_failure_count() {
    let entry = entry("Test::job");
    let run_id = entry.next_run_id();
    let now = SystemTime::now();

    entry.record_start(run_id, JobTrigger::Schedule, now);
    entry.record_finish(run_id, now, JobRunOutcome::Failed("boom".into()));

    assert_eq!(entry.info().failure_count, 1);
}

#[test]
fn recent_runs_are_bounded() {
    let entry = entry("Test::job");

    for _ in 0..100 {
        let run_id = entry.next_run_id();
        let now = SystemTime::now();

        entry.record_start(run_id, JobTrigger::Schedule, now);
        entry.record_finish(run_id, now, JobRunOutcome::Success);
    }

    assert!(entry.recent_runs().len() <= super::RECENT_RUNS_CAP);
    assert_eq!(entry.info().run_count, 100);
}

#[test]
fn pause_and_resume_toggle_state() {
    let entry = entry("Test::job");

    assert!(entry.pause());
    assert!(!entry.pause(), "pause is idempotent");
    assert_eq!(entry.info().state, JobState::Paused);
    assert!(entry.is_paused());

    assert!(entry.resume());
    assert!(!entry.resume(), "resume is idempotent");
    assert_eq!(entry.info().state, JobState::Scheduled);
    assert!(!entry.is_paused());
}

#[test]
fn skipped_firings_are_counted() {
    let entry = entry("Test::job");

    entry.record_skipped();
    entry.record_skipped();

    assert_eq!(entry.info().skipped_count, 2);
}

#[test]
fn begin_cancel_is_terminal() {
    let entry = entry("Test::job");

    entry.begin_cancel();

    assert_eq!(entry.info().state, JobState::Cancelling);
    assert!(entry.token.is_cancelled());

    // A late run-finish must not resurrect a cancelling job to Scheduled.
    let run_id = entry.next_run_id();
    entry.record_finish(run_id, SystemTime::now(), JobRunOutcome::Success);

    assert_eq!(entry.info().state, JobState::Cancelling);
}

#[test]
fn registry_lookup_by_id_and_name() {
    let registry = JobRegistry::default();
    let entry = entry("Reaper::sweep");
    let id = entry.id;

    registry.insert(Arc::clone(&entry));

    assert!(registry.get(id).is_some());
    assert!(registry.by_name("Reaper::sweep").is_some());
    assert!(registry.by_name("missing").is_none());
    assert_eq!(registry.entries().len(), 1);

    registry.remove(id);

    assert!(registry.is_empty());
}
