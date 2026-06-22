//! `#[component(by_value)]`: a component stored and injected as `Self` rather
//! than `Arc<Self>`, for cheap-to-clone (internally-`Arc`) types.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use overseer::{Daemon, component};

/// Internally `Arc`, so cloning is cheap and shares the counter. `#[default]`
/// keeps the field as owned state rather than an injected dependency.
#[component(by_value)]
#[derive(Clone)]
struct Pool {
    #[default]
    hits: Arc<AtomicU32>,
}

/// Injects `Pool` by value (no `Arc<Pool>` wrapper).
#[component]
struct Service {
    pool: Pool,
}

#[tokio::test]
async fn by_value_component_is_stored_and_injected_unwrapped() {
    let daemon = Daemon::builder("by-value-test")
        .auto_discover()
        .build()
        .await
        .expect("daemon builds");

    // `get` returns the handle: `Pool` by value, not `Arc<Pool>`. The type
    // annotation is the proof — it would not compile if the handle were wrapped.
    let pool: Pool = daemon.container.get::<Pool>().expect("Pool constructed");
    let service = daemon.container.get::<Service>().expect("Service constructed");

    // The injected `Pool` is a cheap clone sharing the one inner `Arc` counter:
    // bumping it through the service is observed through the container's handle.
    service.pool.hits.fetch_add(1, Ordering::SeqCst);

    assert_eq!(
        pool.hits.load(Ordering::SeqCst),
        1,
        "by-value clones share the internal Arc state"
    );
}
