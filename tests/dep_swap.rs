//! End-to-end proof of the `Dep<T>` live-slot model: a component injected as
//! `Dep<T>` in two independent consumers shares one swappable slot, so replacing
//! the instance through the container is observed by both consumers, while an
//! `Arc<T>` snapshot taken before the swap stays pinned to the old instance.

use std::sync::Arc;

use overseerd::{App, Dep, component};

/// The reloadable target. `id` distinguishes the original instance (built with
/// `Default`) from a swapped-in replacement.
#[component]
struct Shared {
    #[default]
    id: usize,
}

impl Shared {
    fn id(&self) -> usize {
        self.id
    }
}

/// First consumer holding a live handle to [`Shared`].
#[component]
struct ConsumerA {
    shared: Dep<Shared>,
}

impl ConsumerA {
    fn shared(&self) -> &Dep<Shared> {
        &self.shared
    }
}

/// Second, independent consumer holding its own live handle to [`Shared`].
#[component]
struct ConsumerB {
    shared: Dep<Shared>,
}

impl ConsumerB {
    fn shared(&self) -> &Dep<Shared> {
        &self.shared
    }
}

#[tokio::test]
async fn dep_swap_is_shared_across_two_injection_sites() {
    let daemon = App::builder("dep-swap")
        .auto_discover()
        .build()
        .await
        .expect("daemon builds");

    let a = daemon
        .container()
        .get::<ConsumerA>()
        .expect("ConsumerA constructed");
    let b = daemon
        .container()
        .get::<ConsumerB>()
        .expect("ConsumerB constructed");

    let a_before = a.shared().snapshot();
    let b_before = b.shared().snapshot();

    assert!(
        Arc::ptr_eq(&a_before, &b_before),
        "both Dep injection sites must resolve to the same shared instance"
    );
    assert_eq!(a_before.id(), 0, "the original instance uses Default");

    let reloader = daemon
        .container()
        .resolve::<Dep<Shared>>()
        .await
        .expect("Dep<Shared> resolves from the container");

    assert!(
        Arc::ptr_eq(&reloader.snapshot(), &a_before),
        "a freshly resolved Dep shares the same slot as the injected ones"
    );

    reloader.replace(Arc::new(Shared { id: 42 }));

    let a_after = a.shared().snapshot();
    let b_after = b.shared().snapshot();

    assert!(
        Arc::ptr_eq(&a_after, &b_after),
        "after the swap both sites still share one instance"
    );
    assert!(
        !Arc::ptr_eq(&a_before, &a_after),
        "the slot was actually swapped, not mutated in place"
    );
    assert_eq!(a_after.id(), 42, "ConsumerA observes the swapped-in value");
    assert_eq!(b_after.id(), 42, "ConsumerB observes the swapped-in value");
    assert_eq!(
        a.shared().get().id(),
        42,
        "the guard read also observes the new generation"
    );

    assert_eq!(
        a_before.id(),
        0,
        "a snapshot taken before the swap stays pinned to the old instance"
    );
}
