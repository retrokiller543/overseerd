use std::{any::TypeId, collections::HashMap, future::Future, pin::Pin, sync::Arc};

use overseerd_core::{ResolverSet, ScopeId, StaticScope, TypeDescriptor};

use super::*;
use crate::{
    BoxedComponent, ComponentDescriptor, ComponentFactoryDescriptor, ScopeRegistry,
    descriptors::component::from_boxed,
};

const VISIBLE_SCOPE_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/fresh-visible");
const SIBLING_SCOPE_ID: ScopeId = overseerd_core::namespaced_id!(ScopeId, "test/fresh-sibling");

/// The branch from which fresh providers are resolved.
struct VisibleScope;

impl StaticScope for VisibleScope {
    const ID: ScopeId = VISIBLE_SCOPE_ID;
    const RANK: u8 = 1;
    const NAME: &'static str = "Visible";
}

/// An inaccessible sibling of [`VisibleScope`].
struct SiblingScope;

impl StaticScope for SiblingScope {
    const ID: ScopeId = SIBLING_SCOPE_ID;
    const RANK: u8 = 1;
    const NAME: &'static str = "Sibling";
}

/// Trait used to distinguish fresh providers across sibling branches.
trait BranchProvider: Send + Sync {
    fn source(&self) -> &'static str;
}

/// The provider declared in the visible branch.
struct VisibleProvider;

impl BranchProvider for VisibleProvider {
    fn source(&self) -> &'static str {
        "visible"
    }
}

/// The globally primary provider declared in the inaccessible sibling branch.
struct SiblingPrimaryProvider;

impl BranchProvider for SiblingPrimaryProvider {
    fn source(&self) -> &'static str {
        "sibling"
    }
}

fn no_dependencies() -> Vec<DependencyDescriptor> {
    Vec::new()
}

fn visible_factory<'a>(
    _: &'a mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = crate::Result<BoxedComponent>> + Send + 'a>> {
    Box::pin(async {
        let handle = Arc::new(VisibleProvider);

        Ok(BoxedComponent {
            ty: TypeDescriptor::of::<VisibleProvider>("VisibleProvider"),
            value: Box::new(Injectable::into_stored(handle)),
        })
    })
}

fn sibling_factory<'a>(
    _: &'a mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = crate::Result<BoxedComponent>> + Send + 'a>> {
    Box::pin(async {
        let handle = Arc::new(SiblingPrimaryProvider);

        Ok(BoxedComponent {
            ty: TypeDescriptor::of::<SiblingPrimaryProvider>("SiblingPrimaryProvider"),
            value: Box::new(Injectable::into_stored(handle)),
        })
    })
}

static VISIBLE_FACTORY: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
    construct: visible_factory,
    dependencies: no_dependencies,
    default: false,
}];

static SIBLING_FACTORY: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
    construct: sibling_factory,
    dependencies: no_dependencies,
    default: false,
}];

fn visible_factories() -> &'static [ComponentFactoryDescriptor] {
    &VISIBLE_FACTORY
}

fn sibling_factories() -> &'static [ComponentFactoryDescriptor] {
    &SIBLING_FACTORY
}

fn visible_descriptor() -> ComponentDescriptor {
    ComponentDescriptor {
        id: "visible-provider",
        name: "VisibleProvider",
        ty: TypeDescriptor::of::<VisibleProvider>("VisibleProvider"),
        scope: &VisibleScope,
        factories: visible_factories,
        hooks: overseerd_hooks::no_hooks,
    }
}

fn sibling_descriptor() -> ComponentDescriptor {
    ComponentDescriptor {
        id: "sibling-primary-provider",
        name: "SiblingPrimaryProvider",
        ty: TypeDescriptor::of::<SiblingPrimaryProvider>("SiblingPrimaryProvider"),
        scope: &SiblingScope,
        factories: sibling_factories,
        hooks: overseerd_hooks::no_hooks,
    }
}

fn erase_visible(component: &BoxedComponent) -> BoxedComponent {
    let concrete = from_boxed::<Arc<VisibleProvider>>(component).expect("visible provider stored");
    let erased: Arc<dyn BranchProvider> = concrete;

    BoxedComponent {
        ty: TypeDescriptor::of::<dyn BranchProvider>("dyn BranchProvider"),
        value: Box::new(Injectable::into_stored(erased)),
    }
}

fn erase_sibling(component: &BoxedComponent) -> BoxedComponent {
    let concrete =
        from_boxed::<Arc<SiblingPrimaryProvider>>(component).expect("sibling provider stored");
    let erased: Arc<dyn BranchProvider> = concrete;

    BoxedComponent {
        ty: TypeDescriptor::of::<dyn BranchProvider>("dyn BranchProvider"),
        value: Box::new(Injectable::into_stored(erased)),
    }
}

fn branch_provider(
    concrete_ty: TypeDescriptor,
    primary: bool,
    erase: fn(&BoxedComponent) -> BoxedComponent,
) -> ProviderDescriptor {
    ProviderDescriptor {
        trait_ty: TypeDescriptor::of::<dyn BranchProvider>("dyn BranchProvider"),
        concrete_ty,
        qualifier: "shared",
        primary,
        priority: 0,
        ordering: &[],
        erase,
    }
}

async fn sibling_branches() -> (Arc<ScopeContainer>, Arc<ScopeContainer>) {
    let visible = visible_descriptor();
    let sibling = sibling_descriptor();
    let factory_backed = [visible, sibling]
        .into_iter()
        .map(|descriptor| (descriptor.ty.type_id, descriptor))
        .collect::<HashMap<TypeId, ComponentDescriptor>>();
    let providers = vec![
        branch_provider(sibling.ty, true, erase_sibling),
        branch_provider(visible.ty, false, erase_visible),
    ];
    let registry = Arc::new(ScopeRegistry::new(
        HashMap::new(),
        factory_backed,
        providers,
        HashMap::new(),
    ));
    let root =
        ScopeContainer::build_root(&[], Vec::new(), ResolverSet::new(), Arc::clone(&registry))
            .await
            .expect("root builds");
    let visible_branch = ScopeContainer::open_child(
        &VisibleScope,
        Arc::clone(&root),
        Arc::clone(&registry),
        &[],
        Vec::new(),
    )
    .await
    .expect("visible branch opens");
    let sibling_branch = ScopeContainer::open_child(&SiblingScope, root, registry, &[], Vec::new())
        .await
        .expect("sibling branch opens");

    (visible_branch, sibling_branch)
}

#[test]
fn deferred_panics_before_scope_hydration() {
    let deferred = Deferred::<u8>::capture(ScopeResolverSlot::default(), None)
        .expect("deferred slot registers");

    assert!(deferred.try_get().is_none());
    assert!(std::panic::catch_unwind(|| deferred.get()).is_err());
}

#[tokio::test]
async fn unattached_scope_returns_typed_error_for_lazy() {
    let lazy = Lazy::<Arc<u8>>::capture(ScopeResolverSlot::default());

    assert!(matches!(
        lazy.get_or_create().await,
        Err(Error::ScopeUnavailable)
    ));
}

#[tokio::test]
async fn fresh_ignores_primary_provider_from_inaccessible_sibling() {
    let (visible_branch, sibling_branch) = sibling_branches().await;
    let provider = fresh_arc::<dyn BranchProvider>(&visible_branch, None)
        .await
        .expect("fresh resolution succeeds")
        .expect("visible provider resolves");

    assert_eq!(visible_branch.scope().id(), VISIBLE_SCOPE_ID);
    assert_eq!(sibling_branch.scope().id(), SIBLING_SCOPE_ID);
    assert_eq!(provider.source(), "visible");
}

#[tokio::test]
async fn qualified_fresh_selects_repeated_qualifier_from_visible_sibling() {
    let (visible_branch, sibling_branch) = sibling_branches().await;
    let provider = fresh_arc::<dyn BranchProvider>(&visible_branch, Some("shared"))
        .await
        .expect("qualified fresh resolution succeeds")
        .expect("visible qualified provider resolves");

    assert_eq!(visible_branch.scope().id(), VISIBLE_SCOPE_ID);
    assert_eq!(sibling_branch.scope().id(), SIBLING_SCOPE_ID);
    assert_eq!(provider.source(), "visible");
}
