use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use upwell_core::{ResolverSet, ScopeId, TypeDescriptor};

use super::*;
use crate::registry::selection::select_single_provider;

/// A throwaway intermediate scope for exercising child-container construction
/// without depending on any protocol's concrete scopes.
struct TestScope;
struct SameNameScope;

impl Scope for TestScope {
    fn id(&self) -> ScopeId {
        ScopeId::new("test/container").expect("valid test scope ID")
    }

    fn rank(&self) -> u8 {
        1
    }

    fn name(&self) -> &'static str {
        "Test"
    }
}

impl Scope for SameNameScope {
    fn id(&self) -> ScopeId {
        ScopeId::new("test/same-name").expect("valid test scope ID")
    }

    fn rank(&self) -> u8 {
        1
    }

    fn name(&self) -> &'static str {
        "Test"
    }
}

fn registry() -> Arc<ScopeRegistry> {
    Arc::new(
        ScopeRegistry::new(HashMap::new(), HashMap::new(), Vec::new(), HashMap::new())
            .expect("empty scope registry validates"),
    )
}

async fn root() -> Arc<ScopeContainer> {
    ScopeContainer::build_root(&[], Vec::new(), ResolverSet::new(), registry())
        .await
        .expect("root builds")
}

#[tokio::test]
async fn empty_child_scope_retains_its_identity() {
    let root = root().await;
    let registry = registry();

    let child = ScopeContainer::open_child(
        &TestScope,
        Arc::clone(&root),
        Arc::clone(&registry),
        &[],
        Vec::new(),
    )
    .await
    .expect("open child");

    assert!(!Arc::ptr_eq(&root, &child));
    assert_eq!(child.scope().id(), TestScope.id());
    assert!(child.belongs_to_registry(&registry));
    assert!(!child.belongs_to_registry(&self::registry()));
    assert!(!child.can_access(&SameNameScope));
    assert!(matches!(
        child.slot.state,
        ScopeResolverSlotState::Attached(_)
    ));
    assert!(Arc::ptr_eq(
        &child,
        &child.slot.resolve().expect("attached scope resolves")
    ));

    let source = child
        .resolvers()
        .get_arc::<ComponentSource>()
        .expect("empty child exposes its component source");

    assert!(Arc::ptr_eq(
        &child,
        &source.container.upgrade().expect("child remains alive")
    ));
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
        type_id: counted_concrete_id(),
    };
    let provider = ProviderDescriptor {
        trait_ty: TypeDescriptor::of::<dyn Send>("dyn Send"),
        concrete_ty,
        qualifier: "counted",
        primary: false,
        priority: 0,
        ordering: &[],
        erase: erase_unreachable,
    };
    let component = ComponentDescriptor::manual("counted", "Counted", concrete_ty, &Singleton);
    let registry = ScopeRegistry::new(
        HashMap::new(),
        HashMap::from([(component.ty.type_id, component)]),
        vec![provider; PROVIDERS],
        HashMap::new(),
    )
    .expect("provider component is registered");

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

fn test_provider(name: &'static str, qualifier: &'static str, primary: bool) -> ProviderDescriptor {
    ProviderDescriptor {
        trait_ty: TypeDescriptor::of::<dyn Send>("dyn Send"),
        concrete_ty: TypeDescriptor::of::<u8>(name),
        qualifier,
        primary,
        priority: 0,
        ordering: &[],
        erase: erase_unreachable,
    }
}

#[test]
fn single_provider_selection_is_shared_and_ambiguity_aware() {
    let sole = [test_provider("Sole", "sole", false)];

    assert_eq!(
        select_single_provider(&sole).map(|p| p.qualifier),
        Some("sole")
    );

    let two_plain = [
        test_provider("First", "first", false),
        test_provider("Second", "second", false),
    ];

    assert!(select_single_provider(&two_plain).is_none());

    let unique_primary = [
        test_provider("First", "first", true),
        test_provider("Second", "second", false),
    ];

    assert_eq!(
        select_single_provider(&unique_primary).map(|p| p.qualifier),
        Some("first")
    );

    let two_primaries = [
        test_provider("First", "first", true),
        test_provider("Second", "second", true),
    ];

    assert!(select_single_provider(&two_primaries).is_none());
}

#[test]
fn registry_construction_completes_missing_provider_ordinals() {
    let provider = test_provider("Unplanned", "unplanned", false);
    let component =
        ComponentDescriptor::manual("unplanned", "Unplanned", provider.concrete_ty, &Singleton);
    let registry = ScopeRegistry::new(
        HashMap::new(),
        HashMap::from([(component.ty.type_id, component)]),
        vec![provider],
        HashMap::new(),
    )
    .expect("provider component is registered");

    // Previously this expect() panicked for providers missing from the plan.
    assert_eq!(registry.provider_ordinal(&provider), 0);
}

#[test]
fn registry_indexes_follow_final_provider_order_for_first_wins_lookups() {
    let first = ProviderDescriptor {
        trait_ty: TypeDescriptor::of::<dyn Send>("dyn Send"),
        concrete_ty: TypeDescriptor::of::<u8>("First"),
        qualifier: "shared",
        primary: false,
        priority: 20,
        ordering: &[],
        erase: erase_unreachable,
    };
    let winning = ProviderDescriptor {
        trait_ty: TypeDescriptor::of::<dyn Send>("dyn Send"),
        concrete_ty: TypeDescriptor::of::<u16>("Winning"),
        qualifier: "shared",
        primary: false,
        priority: -10,
        ordering: &[],
        erase: erase_unreachable,
    };
    let order = HashMap::from([(
        TypeId::of::<dyn Send>(),
        HashMap::from([(TypeId::of::<u8>(), 1), (TypeId::of::<u16>(), 0)]),
    )]);
    let components = [first, winning]
        .into_iter()
        .map(|provider| {
            let component = ComponentDescriptor::manual(
                provider.qualifier,
                provider.concrete_ty.name,
                provider.concrete_ty,
                &Singleton,
            );

            (component.ty.type_id, component)
        })
        .collect();
    let registry = ScopeRegistry::new(HashMap::new(), components, vec![first, winning], order)
        .expect("provider components are registered");

    assert_eq!(
        registry
            .selection
            .select_global(TypeId::of::<dyn Send>(), Some("shared"))
            .map(|provider| provider.concrete_ty.type_id),
        Some(TypeId::of::<u16>())
    );
    assert_eq!(
        registry
            .selection
            .providers_for_trait(TypeId::of::<dyn Send>())
            .iter()
            .map(|provider| provider.concrete_ty.type_id)
            .collect::<Vec<_>>(),
        [TypeId::of::<u16>(), TypeId::of::<u8>()]
    );
}

#[test]
fn registry_construction_rejects_orphan_provider() {
    let provider = test_provider("Orphan", "orphan", false);
    let error = match ScopeRegistry::new(
        HashMap::new(),
        HashMap::new(),
        vec![provider],
        HashMap::new(),
    ) {
        Ok(_) => panic!("orphan provider must be rejected"),
        Err(error) => error,
    };

    assert!(matches!(error, Error::ProviderComponentMissing(_)));
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
    type Handle = Arc<Self>;

    const ID: &'static str = "never-registered";
    const NAME: &'static str = "NeverRegistered";

    fn into_handle(self) -> Self::Handle {
        Arc::new(self)
    }
}
