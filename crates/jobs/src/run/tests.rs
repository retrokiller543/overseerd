use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use super::{JobProgress, JobRunContext, jitter_delay};
use crate::registry::{JobId, JobRunId};

#[tokio::test]
async fn progress_updates_the_shared_slot() {
    let slot = Arc::new(Mutex::new(None));
    let cx = JobRunContext::new(
        JobRunId::from_raw(7),
        JobId::from_raw(1),
        Arc::from("Test::job"),
        Arc::clone(&slot),
    );

    cx.progress(JobProgress::phase("loading").counted(3, 10))
        .await;

    let snapshot = slot.lock().unwrap().clone().expect("progress recorded");

    assert_eq!(snapshot.phase.as_deref(), Some("loading"));
    assert_eq!(snapshot.current, Some(3));
    assert_eq!(snapshot.total, Some(10));
}

#[test]
fn progress_builders_compose() {
    let progress = JobProgress::phase("indexing")
        .with_message("halfway")
        .counted(50, 100);

    assert_eq!(progress.phase.as_deref(), Some("indexing"));
    assert_eq!(progress.message.as_deref(), Some("halfway"));
    assert_eq!(progress.current, Some(50));
}

#[test]
fn jitter_is_bounded_by_the_configured_span() {
    let jitter = Duration::from_millis(100);

    for run in 0..1000 {
        let delay = jitter_delay(jitter, JobRunId::from_raw(run));

        assert!(delay < jitter, "jitter {delay:?} exceeded span {jitter:?}");
    }
}
