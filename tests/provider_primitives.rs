//! End-to-end coverage for scope-capturing DI provider primitives.

use std::{
    any::TypeId,
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use overseerd::daemon::App;
use overseerd::{
    ComponentDescriptor, Deferred, Descriptor, Fresh, Lazy, ResolverSet, ScopeContainer,
    ScopeRegistry, component, injectable,
};

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

/// One side of a legitimate construction cycle, broken by deferred hydration.
#[component]
struct DeferredCycleA {
    b: Deferred<DeferredCycleB>,
}

/// The eager side of the cycle retains the first component normally.
#[component]
struct DeferredCycleB {
    a: Arc<DeferredCycleA>,
}

/// A transient component holding a deferred dependency on a singleton.
#[component(scope = overseerd::scope::Transient)]
struct TransientDeferredConsumer {
    target: Deferred<PrimitiveTarget>,
}

/// Resolves the transient during root construction through an optional edge, so
/// its deferred handle must hydrate when the root scope attaches.
#[component]
struct RootBuildTransientOwner {
    transient: Option<Arc<TransientDeferredConsumer>>,
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

    let deferred = consumer.deferred.get();
    assert!(Arc::ptr_eq(&deferred, &canonical));
    assert!(Arc::ptr_eq(&consumer.deferred.get(), &canonical));

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
    assert_eq!(consumer.deferred_qualified.get().name(), "second");
}

#[tokio::test]
async fn deferred_hydrates_after_construction_without_retaining_a_cycle() {
    let components = [
        <DeferredCycleA as Descriptor<ComponentDescriptor>>::DESCRIPTOR,
        <DeferredCycleB as Descriptor<ComponentDescriptor>>::DESCRIPTOR,
    ];
    let registry = Arc::new(ScopeRegistry::new(
        HashMap::new(),
        components
            .iter()
            .map(|component| (component.ty.type_id, *component))
            .collect::<HashMap<TypeId, ComponentDescriptor>>(),
        Vec::new(),
        HashMap::new(),
    ));
    let container =
        ScopeContainer::build_root(&components, Vec::new(), ResolverSet::new(), registry)
            .await
            .expect("deferred cycle builds");
    let a = container.get::<DeferredCycleA>().expect("cycle A resolves");
    let b = container.get::<DeferredCycleB>().expect("cycle B resolves");
    let weak_a = Arc::downgrade(&a);
    let weak_b = Arc::downgrade(&b);

    assert!(Arc::ptr_eq(&a.b.get(), &b));
    assert!(Arc::ptr_eq(&b.a, &a));

    drop(a);
    drop(b);
    drop(container);

    assert!(weak_a.upgrade().is_none());
    assert!(weak_b.upgrade().is_none());
}

#[tokio::test]
async fn deferred_in_transient_built_during_root_build_hydrates_at_attach() {
    let app = App::builder("root-build-transient-deferred")
        .auto_discover()
        .build()
        .await
        .expect("app builds with a root-built transient holding a deferred");
    let owner = app
        .container()
        .get::<RootBuildTransientOwner>()
        .expect("owner resolves");
    let canonical = app
        .container()
        .get::<PrimitiveTarget>()
        .expect("canonical target resolves");
    let transient = owner
        .transient
        .as_ref()
        .expect("optional transient constructed during root build");

    // The transient was constructed while the root scope was still building;
    // its deferred handle must have hydrated when the root attached.
    assert!(Arc::ptr_eq(&transient.target.get(), &canonical));
}
