//! Scope-local provider selection: a child scope's own provider takes precedence
//! over a parent's globally primary provider, and the build order must reflect it.

use std::{
    any::TypeId,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use overseerd::{
    ComponentDescriptor, Descriptor, PROVIDERS, ResolverSet, ScopeContainer, ScopeRegistry,
    StaticScope, component, injectable, topological_sort,
};

/// A throwaway child scope for scope-local provider selection tests.
struct ChildScope;

impl StaticScope for ChildScope {
    const RANK: u8 = 1;
    const NAME: &'static str = "Child";
}

/// A trait with providers in two scopes, exercising scope-local selection.
#[injectable]
trait ScopedChoice: Send + Sync {
    fn name(&self) -> &'static str;
}

/// The parent-scope provider, globally primary.
#[component(provide = dyn ScopedChoice, primary)]
struct ParentPrimaryChoice;

impl ScopedChoice for ParentPrimaryChoice {
    fn name(&self) -> &'static str {
        "parent"
    }
}

/// The child scope's own sole provider: it takes precedence over the parent's
/// primary for child-scope consumers at runtime.
#[component(scope = ChildScope, provide = dyn ScopedChoice)]
struct ChildLocalChoice;

impl ScopedChoice for ChildLocalChoice {
    fn name(&self) -> &'static str {
        "child"
    }
}

/// A child-scope consumer of the trait's single provider.
#[component(scope = ChildScope)]
struct ScopedChoiceConsumer {
    chosen: Arc<dyn ScopedChoice>,
}

#[tokio::test]
async fn child_scope_consumer_waits_for_its_scope_local_provider() {
    let parent = <ParentPrimaryChoice as Descriptor<ComponentDescriptor>>::DESCRIPTOR;
    let local = <ChildLocalChoice as Descriptor<ComponentDescriptor>>::DESCRIPTOR;
    let consumer = <ScopedChoiceConsumer as Descriptor<ComponentDescriptor>>::DESCRIPTOR;
    let components = [parent, local, consumer];
    let providers: Vec<_> = PROVIDERS
        .iter()
        .filter(|provider| provider.trait_ty.type_id == TypeId::of::<dyn ScopedChoice>())
        .copied()
        .collect();
    let registry = Arc::new(ScopeRegistry::new(
        HashMap::new(),
        components
            .iter()
            .map(|component| (component.ty.type_id, *component))
            .collect::<HashMap<TypeId, ComponentDescriptor>>(),
        providers.clone(),
        HashMap::new(),
    ));

    let root =
        ScopeContainer::build_root(&[parent], Vec::new(), ResolverSet::new(), registry.clone())
            .await
            .expect("root builds");
    let prebuilt: HashSet<TypeId> = [parent.ty.type_id].into_iter().collect();

    // The consumer is listed first on purpose: the sort must move it after the
    // child-scope provider it actually captures, not treat the globally primary
    // (already-built) parent provider as its wait set.
    let child_components = [consumer, local];
    let order = topological_sort(&child_components, &prebuilt, &providers, &HashMap::new())
        .expect("child scope sorts");
    let names: Vec<_> = order.iter().map(|component| component.name).collect();

    assert_eq!(names, ["ChildLocalChoice", "ScopedChoiceConsumer"]);

    let order: Vec<ComponentDescriptor> = order.into_iter().copied().collect();
    let child = ScopeContainer::open_child(&ChildScope, root, registry, &order, Vec::new())
        .await
        .expect("child scope builds");

    assert_eq!(
        child
            .get::<ScopedChoiceConsumer>()
            .expect("consumer built")
            .chosen
            .name(),
        "child",
        "the consumer captures the scope-local provider, not the parent primary"
    );
}
