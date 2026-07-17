use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::stop_reload_triggers;

struct ActiveTask(Arc<AtomicUsize>);

impl Drop for ActiveTask {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn stopping_triggers_waits_until_every_task_has_dropped() {
    let active = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    for _ in 0..4 {
        let active = Arc::clone(&active);

        handles.push(tokio::spawn(async move {
            active.fetch_add(1, Ordering::SeqCst);
            let _active = ActiveTask(Arc::clone(&active));

            std::future::pending::<()>().await;
        }));
    }

    while active.load(Ordering::SeqCst) != 4 {
        tokio::task::yield_now().await;
    }

    stop_reload_triggers(handles).await;

    assert_eq!(active.load(Ordering::SeqCst), 0, "no trigger task lingers");
}
