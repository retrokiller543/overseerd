use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use overseerd_core::{ResolverSet, TypeDescriptor};

use super::*;

/// A throwaway intermediate scope for exercising child-container construction
/// without depending on any protocol's concrete scopes.
struct TestScope;

impl Scope for TestScope {
    fn rank(&self) -> u8 {
        1
    }

    fn name(&self) -> &'static str {
        "Test"
    }
}

fn registry() -> Arc<ScopeRegistry> {
    Arc::new(ScopeRegistry::new(HashMap::new(), Vec::new()))
}

async fn root() -> Arc<ScopeContainer> {
    ScopeContainer::build_root(&[], Vec::new(), ResolverSet::new(), registry())
        .await
        .expect("root builds")
}

#[tokio::test]
async fn empty_child_scope_is_skipped() {
    let root = root().await;

    let child =
        ScopeContainer::open_child(&TestScope, Arc::clone(&root), registry(), &[], Vec::new())
            .await
            .expect("open child");

    assert!(
        Arc::ptr_eq(&root, &child),
        "empty child scope should reuse the parent container"
    );
}

#[tokio::test]
async fn child_scope_with_a_seed_is_built() {
    let root = root().await;

    let seed = BoxedComponent {
        ty: TypeDescriptor::of::<u8>("u8"),
        value: Box::new(7u8),
    };

    let child =
        ScopeContainer::open_child(&TestScope, Arc::clone(&root), registry(), &[], vec![seed])
            .await
            .expect("open child");

    assert!(
        !Arc::ptr_eq(&root, &child),
        "a seeded scope should allocate its own container"
    );
    assert_eq!(child.scope().name(), "Test");
}

static CONCRETE_ID_CALLS: AtomicUsize = AtomicUsize::new(0);

fn counted_concrete_id() -> TypeId {
    CONCRETE_ID_CALLS.fetch_add(1, Ordering::Relaxed);

    TypeId::of::<u8>()
}

fn erase_unreachable(_: &BoxedComponent) -> BoxedComponent {
    panic!("the provider index test never instantiates a provider")
}

#[test]
fn provider_lookup_uses_the_prebuilt_concrete_index() {
    const PROVIDERS: usize = 256;

    let concrete_ty = TypeDescriptor {
        name: "Counted",
        type_name: std::any::type_name::<u8>,
        type_id: counted_concrete_id,
    };
    let provider = ProviderDescriptor {
        trait_ty: TypeDescriptor::of::<dyn Send>("dyn Send"),
        concrete_ty,
        qualifier: "counted",
        primary: false,
        erase: erase_unreachable,
    };
    let registry = ScopeRegistry::new(HashMap::new(), vec![provider; PROVIDERS]);

    CONCRETE_ID_CALLS.store(0, Ordering::SeqCst);

    for _ in 0..1_000 {
        assert_eq!(registry.providers_for(TypeId::of::<u8>()).len(), PROVIDERS);
    }

    assert_eq!(
        CONCRETE_ID_CALLS.load(Ordering::SeqCst),
        0,
        "provider lookup rescanned concrete descriptor functions"
    );
}

#[tokio::test]
async fn component_source_does_not_keep_a_scope_alive() {
    let root = root().await;
    let source = root
        .resolvers()
        .get_arc::<ComponentSource>()
        .expect("component source installed");

    drop(root);

    assert!(
        source.component::<NeverRegistered>().is_none(),
        "a weak component source must not retain its container"
    );
}

struct NeverRegistered;

impl Component for NeverRegistered {
    const ID: &'static str = "never-registered";
    const NAME: &'static str = "NeverRegistered";
    type Handle = Arc<Self>;

    fn into_handle(self) -> Self::Handle {
        Arc::new(self)
    }
}
