use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::registry::{JobId, JobInfo, JobState};
use crate::schedule::ScheduleInfo;

fn info(state: JobState, next_run_at: Option<SystemTime>) -> JobInfo {
    JobInfo {
        id: JobId::from_raw(0),
        name: Arc::from("Test::job"),
        schedule: ScheduleInfo::Every(Duration::from_secs(60)),
        state,
        next_run_at,
        last_run: None,
        progress: None,
        run_count: 0,
        failure_count: 0,
        skipped_count: 0,
        labels: BTreeMap::new(),
        description: None,
    }
}

#[test]
fn schedule_lag_reports_overdue_scheduled_jobs() {
    let past = SystemTime::now() - Duration::from_secs(30);
    let info = info(JobState::Scheduled, Some(past));

    let lag = info.schedule_lag().expect("overdue job has lag");

    assert!(lag >= Duration::from_secs(29));
    assert!(info.is_stale(Duration::from_secs(10)));
}

#[test]
fn paused_jobs_are_never_stale_even_with_a_past_next_run() {
    // `pause` marks the job paused synchronously while the loop clears `next_run_at` later, so a
    // paused job may still carry a past `next_run_at`. It must not be reported as stale.
    let past = SystemTime::now() - Duration::from_secs(300);
    let info = info(JobState::Paused, Some(past));

    assert_eq!(info.schedule_lag(), None);
    assert!(!info.is_stale(Duration::from_secs(1)));
}
