//! Tests for the tokio-tungstenite request/reply actor's pending-call pruning.

use std::collections::HashMap;

use tokio::sync::oneshot;

use super::{Pending, prune_pending};

#[test]
fn prune_pending_removes_calls_after_receiver_is_dropped() {
    let (closed_tx, closed_rx) = oneshot::channel();
    let (open_tx, _open_rx) = oneshot::channel();
    let mut pending: HashMap<u64, Pending> = HashMap::new();

    pending.insert(1, closed_tx);
    pending.insert(2, open_tx);
    drop(closed_rx);

    prune_pending(&mut pending);

    assert!(!pending.contains_key(&1));
    assert!(pending.contains_key(&2));
}
