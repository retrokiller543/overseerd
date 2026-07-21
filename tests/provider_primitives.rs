//! End-to-end coverage for scope-capturing DI provider primitives.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use overseerd::daemon::App;
use overseerd::{Deferred, Fresh, Lazy, component, injectable};

static IDS: AtomicU64 = AtomicU64::new(1);

/// A factory-backed singleton with observable reconstruction identity.
#[component]
struct PrimitiveTarget {
    #[default]
    id: Identity,
}

/// A unique identity assigned by each factory run.
struct Identity(u64);

impl Default for Identity {
    fn default() -> Self {
        Self(IDS.fetch_add(1, Ordering::Relaxed))
    }
}

/// Trait projection used to cover qualified and collection fresh construction.
#[injectable]
trait PrimitiveProvider: Send + Sync {
    fn name(&self) -> &'static str;
}

impl PrimitiveProvider for PrimitiveTarget {
    fn name(&self) -> &'static str {
        "target"
    }
}

/// A second provider establishing deterministic fresh collection order.
#[component(provide = dyn PrimitiveProvider, qualifier = "second")]
struct SecondPrimitiveProvider;

impl PrimitiveProvider for SecondPrimitiveProvider {
    fn name(&self) -> &'static str {
        "second"
    }
}

/// Provider registration for the identity-bearing target.
#[component(
    provide = dyn PrimitiveProvider,
    qualifier = "target",
    before = SecondPrimitiveProvider
)]
struct PrimitiveProviderTarget;

impl PrimitiveProvider for PrimitiveProviderTarget {
    fn name(&self) -> &'static str {
        "target-provider"
    }
}

/// Consumer exercising field macro classification and qualifier preservation.
#[component]
struct PrimitiveConsumer {
    lazy: Lazy<Arc<PrimitiveTarget>>,
    fresh: Fresh<Arc<PrimitiveTarget>>,
    deferred: Deferred<PrimitiveTarget>,
    fresh_all: Fresh<Vec<Arc<dyn PrimitiveProvider>>>,
    #[qualifier = "second"]
    lazy_qualified: Lazy<Arc<dyn PrimitiveProvider>>,
    #[qualifier = "target"]
    fresh_qualified: Fresh<Arc<dyn PrimitiveProvider>>,
    #[qualifier = "second"]
    deferred_qualified: Deferred<dyn PrimitiveProvider>,
}

#[tokio::test]
async fn lazy_fresh_and_deferred_follow_their_cache_contracts() {
    let app = App::builder("provider-primitives")
        .auto_discover()
        .build()
        .await
        .expect("app builds");
    let consumer = app
        .container()
        .get::<PrimitiveConsumer>()
        .expect("consumer resolves");
    let canonical = app
        .container()
        .get::<PrimitiveTarget>()
        .expect("canonical target resolves");

    assert!(consumer.lazy.get().is_none());
    let lazy = consumer.lazy.get_or_create().await.expect("lazy resolves");
    assert!(Arc::ptr_eq(&lazy, &canonical));
    assert!(Arc::ptr_eq(
        &consumer.lazy.get().expect("lazy caches"),
        &canonical
    ));
    assert!(Arc::ptr_eq(
        &consumer.lazy.create().await.expect("lazy refreshes"),
        &canonical
    ));

    let fresh = consumer.fresh.create().await.expect("fresh constructs");
    assert_ne!(fresh.id.0, canonical.id.0);
    assert!(Arc::ptr_eq(
        &app.container()
            .get::<PrimitiveTarget>()
            .expect("canonical remains"),
        &canonical
    ));

    assert!(consumer.deferred.get().is_none());
    let deferred = consumer
        .deferred
        .get_or_resolve()
        .await
        .expect("deferred resolves");
    assert!(Arc::ptr_eq(&deferred, &canonical));
    assert!(Arc::ptr_eq(
        &consumer.deferred.get().expect("weak cache upgrades"),
        &canonical
    ));

    assert_eq!(
        consumer
            .fresh_all
            .create()
            .await
            .expect("fresh providers construct")
            .iter()
            .map(|provider| provider.name())
            .collect::<Vec<_>>(),
        ["target-provider", "second"]
    );
    assert_eq!(
        consumer
            .lazy_qualified
            .get_or_create()
            .await
            .expect("qualified lazy resolves")
            .name(),
        "second"
    );
    assert_eq!(
        consumer
            .fresh_qualified
            .create()
            .await
            .expect("qualified fresh resolves")
            .name(),
        "target-provider"
    );
    assert_eq!(
        consumer
            .deferred_qualified
            .get_or_resolve()
            .await
            .expect("qualified deferred resolves")
            .name(),
        "second"
    );
}
