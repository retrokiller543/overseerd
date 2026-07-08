use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::{InMemoryJobLogStore, JobLogConfig, JobLogLevel, JobLogRecord, JobLogSink};
use crate::registry::{JobId, JobRunId};

fn record(run: u64, message: &str) -> JobLogRecord {
    JobLogRecord {
        run_id: JobRunId::from_raw(run),
        job_id: JobId::from_raw(0),
        job_name: Arc::from("Test::job"),
        timestamp: SystemTime::now(),
        level: JobLogLevel::Info,
        target: "overseerd::example".to_string(),
        message: message.to_string(),
    }
}

#[tokio::test]
async fn records_are_returned_oldest_first_up_to_limit() {
    let store = InMemoryJobLogStore::with_defaults();

    for i in 0..5 {
        store.record(record(1, &format!("line {i}"))).await;
    }

    let recent = store.records(JobRunId::from_raw(1), 3).await;

    assert_eq!(recent.len(), 3);
    assert_eq!(recent[0].message, "line 2");
    assert_eq!(recent[2].message, "line 4");
}

#[tokio::test]
async fn max_runs_evicts_the_oldest_run() {
    let config = JobLogConfig {
        max_runs: 2,
        ..JobLogConfig::default()
    };
    let store = InMemoryJobLogStore::new(config);

    for run in 0..3 {
        store.record(record(run, "hello")).await;
    }

    assert!(store.records(JobRunId::from_raw(0), 10).await.is_empty());
    assert_eq!(store.records(JobRunId::from_raw(1), 10).await.len(), 1);
    assert_eq!(store.records(JobRunId::from_raw(2), 10).await.len(), 1);
}

#[tokio::test]
async fn max_bytes_per_run_sheds_oldest_records() {
    let config = JobLogConfig {
        max_bytes_per_run: 40,
        ..JobLogConfig::default()
    };
    let store = InMemoryJobLogStore::new(config);

    // Each record is well over 20 bytes (target + message), so the run cannot hold all three.
    for i in 0..3 {
        store
            .record(record(1, &format!("message number {i}")))
            .await;
    }

    let kept = store.records(JobRunId::from_raw(1), 10).await;

    assert!(
        kept.len() < 3,
        "expected oldest records to be shed, kept {}",
        kept.len()
    );
    assert_eq!(kept.last().unwrap().message, "message number 2");
}

#[tokio::test]
async fn a_single_oversized_record_is_retained() {
    let config = JobLogConfig {
        max_bytes_per_run: 8,
        ..JobLogConfig::default()
    };
    let store = InMemoryJobLogStore::new(config);

    // One record far larger than the cap must still be kept — never leave a run with no logs.
    store
        .record(record(1, "a message much larger than eight bytes"))
        .await;

    let kept = store.records(JobRunId::from_raw(1), 10).await;

    assert_eq!(kept.len(), 1);
}

#[tokio::test]
async fn disabled_store_drops_everything() {
    let config = JobLogConfig {
        enabled: false,
        ..JobLogConfig::default()
    };
    let store = InMemoryJobLogStore::new(config);

    store.record(record(1, "ignored")).await;

    assert!(store.records(JobRunId::from_raw(1), 10).await.is_empty());
}

#[tokio::test]
async fn expired_runs_are_dropped_on_access() {
    let config = JobLogConfig {
        ttl: Duration::from_millis(10),
        ..JobLogConfig::default()
    };
    let store = InMemoryJobLogStore::new(config);

    let mut old = record(1, "stale");
    old.timestamp = SystemTime::now() - Duration::from_secs(60);
    store.record(old).await;

    // A later record from another run triggers the retain sweep that drops the expired run.
    store.record(record(2, "fresh")).await;

    assert!(store.records(JobRunId::from_raw(1), 10).await.is_empty());
    assert_eq!(store.records(JobRunId::from_raw(2), 10).await.len(), 1);
}
