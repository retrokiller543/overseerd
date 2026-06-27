use std::{
    any::{Any, TypeId},
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
};

use overseerd_core::{
    Cardinality, Resolver, ResolverCtx, ResolverSet, Scope, Singleton, Transient,
};
use tracing::{debug, error, info, instrument, trace};

use crate::descriptors::BoxedComponent;
use crate::{
    descriptors::{
        Component, ComponentDescriptor, Injectable, ProviderDescriptor,
        component::{ComponentConstructionContext, ScopeStore},
    },
    error::Error,
};

/// Shared, immutable data a [`ScopeContainer`] needs to resolve beyond its own
/// store: the `Transient` components it may construct on demand and the trait
/// providers used to alias instances. Held behind an `Arc` and shared by every
/// scope in an application (root, per-connection, per-request).
pub struct ScopeRegistry {
    /// Transient components keyed by their concrete `TypeId`, for on-demand
    /// construction at resolution time. Transients are never cached.
    transient: HashMap<TypeId, ComponentDescriptor>,
    providers: Vec<ProviderDescriptor>,
}

impl ScopeRegistry {
    pub fn new(
        transient: HashMap<TypeId, ComponentDescriptor>,
        providers: Vec<ProviderDescriptor>,
    ) -> Self {
        Self {
            transient,
            providers,
        }
    }

    pub(crate) fn providers(&self) -> &[ProviderDescriptor] {
        &self.providers
    }

    pub(crate) fn transient(&self, target: TypeId) -> Option<ComponentDescriptor> {
        self.transient.get(&target).copied()
    }
}

/// Constructs a fresh `Transient` instance whose `Injectable::Target` is `H`, if
/// one is registered. A transient is rebuilt on every resolution and never stored,
/// so it is constructed in a throwaway context parented to `parent` (its
/// dependencies — singletons in v1 — resolve up the chain).
pub(crate) async fn construct_transient<H: Injectable>(
    registry: &Arc<ScopeRegistry>,
    parent: Option<Arc<ScopeContainer>>,
) -> Option<H> {
    let target = TypeId::of::<H::Target>();
    let descriptor = registry.transient(target)?;
    let factory = descriptor.effective_factory().ok().flatten()?;

    let externals = parent
        .as_ref()
        .map(|p| p.resolvers().clone())
        .unwrap_or_default();

    let mut cx =
        ComponentConstructionContext::new(&Transient, parent, Arc::clone(registry), externals);

    match (factory.construct)(&mut cx).await {
        Ok(boxed) => crate::descriptors::component::from_boxed::<H>(&boxed),

        Err(e) => {
            error!(component = %descriptor.name, error = %e, "transient construction failed");

            None
        }
    }
}

/// One scope's constructed instances, layered over an optional parent scope.
///
/// The root container is the singleton scope (`parent: None`); a per-connection
/// scope parents the root, and a per-request scope parents the connection.
/// Resolution walks this scope first, then each longer-lived parent, so a request
/// handler sees request-, connection-, and singleton-scoped instances uniformly.
///
/// The container is also a [`ResolverCtx`]: it exposes a [`ComponentSource`] (for
/// resolving components by type — used by hooks and config-targeted resolution) plus
/// any *external* resolvers (the config store) threaded in at build.
pub struct ScopeContainer {
    scope: &'static dyn Scope,
    store: ScopeStore,
    parent: Option<Arc<ScopeContainer>>,
    registry: Arc<ScopeRegistry>,
    resolvers: ResolverSet,
}

impl ResolverCtx for ScopeContainer {
    fn resolver(&self, kind: TypeId) -> Option<&dyn Any> {
        self.resolvers.resolver(kind)
    }
}

/// A [`Resolver`] over a [`ScopeContainer`], resolving components by type.
///
/// Held in the container's own resolver set under a [`Weak`] back-reference (so it
/// adds no reference cycle), it is how a hook reaches its `&self` receiver through
/// the erased `&dyn ResolverCtx` it is handed: `ctx.get::<ComponentSource>()?.component::<Self>()`.
pub struct ComponentSource {
    container: Weak<ScopeContainer>,
}

impl Resolver for ComponentSource {}

impl ComponentSource {
    /// The component of type `C` as its handle (`Arc<C>` or the by-value handle),
    /// resolved through the backing scope and its parents.
    pub fn component<C: Component>(&self) -> Option<C::Handle> {
        self.container.upgrade()?.get::<C>()
    }

    /// The handle `H` resolved through the backing scope and its parents.
    pub fn resolve<H: Injectable>(&self) -> Option<H> {
        self.container.upgrade()?.resolve_built::<H>()
    }
}

impl ScopeContainer {
    /// The scope this container holds.
    pub fn scope(&self) -> &'static dyn Scope {
        self.scope
    }

    /// The external resolvers threaded into this scope (config store, …), shared with
    /// child scopes.
    pub fn resolvers(&self) -> &ResolverSet {
        &self.resolvers
    }

    /// Resolves all registered singleton components in dependency order into the
    /// root container.
    ///
    /// `components` is the singleton-scoped component set (after default/override
    /// resolution). `instances` holds pre-built singletons supplied at the builder;
    /// they are seeded first, so factory-built components may depend on them.
    /// `externals` is the external resolver set (e.g. the config store) the factories
    /// resolve `Cfg<T>` and similar through.
    #[instrument(skip_all, fields(count = components.len()))]
    pub async fn build_root(
        components: &[ComponentDescriptor],
        instances: Vec<BoxedComponent>,
        externals: ResolverSet,
        registry: Arc<ScopeRegistry>,
    ) -> crate::Result<Arc<ScopeContainer>> {
        debug!("resolving singleton dependency order");

        let prebuilt: HashSet<TypeId> = instances.iter().map(|i| (i.ty.type_id)()).collect();

        let order = topological_sort(components, &prebuilt, registry.providers())?;
        let order: Vec<ComponentDescriptor> = order.into_iter().copied().collect();

        let root = Self::build(&Singleton, None, registry, &order, instances, externals).await?;

        info!(count = root.store.components.len(), "root container built");

        Ok(root)
    }

    /// Opens a child scope over `parent`, seeding `seeds` then constructing `order`
    /// (a dependency order precomputed at application build). Every scope — the four
    /// built-ins and any future user-defined one — is created through this single
    /// primitive.
    ///
    /// An **empty** scope (nothing to construct, nothing to seed) holds no state of
    /// its own, so resolution through it would only pass through to `parent`. Rather
    /// than allocate a redundant container, this returns `parent` directly.
    pub async fn open_child(
        scope: &'static dyn Scope,
        parent: Arc<ScopeContainer>,
        registry: Arc<ScopeRegistry>,
        order: &[ComponentDescriptor],
        seeds: Vec<BoxedComponent>,
    ) -> crate::Result<Arc<ScopeContainer>> {
        if order.is_empty() && seeds.is_empty() {
            return Ok(parent);
        }

        let externals = parent.resolvers().clone();

        Self::build(scope, Some(parent), registry, order, seeds, externals).await
    }

    /// Seeds instances, then constructs `order` in sequence, aliasing trait
    /// providers as each instance lands. Resolution during construction reaches the
    /// parent chain. The frozen container is built with [`Arc::new_cyclic`] so it can
    /// hold its own [`ComponentSource`] (a `Weak` self-reference) in its resolver set.
    async fn build(
        scope: &'static dyn Scope,
        parent: Option<Arc<ScopeContainer>>,
        registry: Arc<ScopeRegistry>,
        order: &[ComponentDescriptor],
        seeds: Vec<BoxedComponent>,
        externals: ResolverSet,
    ) -> crate::Result<Arc<ScopeContainer>> {
        let mut cx =
            ComponentConstructionContext::new(scope, parent, Arc::clone(&registry), externals);
        let providers = registry.providers();

        for seed in seeds {
            let type_id = (seed.ty.type_id)();

            cx.insert(seed);
            register_providers_for(&mut cx, providers, type_id);
        }

        for descriptor in order {
            match descriptor.effective_factory()? {
                Some(factory) => {
                    debug!(component = %descriptor.name, scope = scope.name(), "constructing component");

                    let component = (factory.construct)(&mut cx).await?;

                    cx.insert(component);

                    trace!(component = %descriptor.name, "component ready");
                }

                None => {
                    if !cx.contains((descriptor.ty.type_id)()) {
                        error!(component = %descriptor.name, "no instance provided for factory-less component");
                        return Err(Error::MissingComponent(descriptor.name));
                    }

                    trace!(component = %descriptor.name, "using provided instance");
                }
            }

            register_providers_for(&mut cx, providers, (descriptor.ty.type_id)());
        }

        let (scope, store, parent, registry, externals) = cx.into_parts();

        Ok(Arc::new_cyclic(|weak| {
            let mut resolvers = externals;
            resolvers.insert(Arc::new(ComponentSource {
                container: weak.clone(),
            }));

            ScopeContainer {
                scope,
                store,
                parent,
                registry,
                resolvers,
            }
        }))
    }

    /// Returns the registered component of type `T` as its handle (`Arc<T>` by
    /// default, or the by-value handle for a `#[component(by_value)]` type),
    /// resolved through this scope and its parents.
    pub fn get<T: Component>(&self) -> Option<T::Handle> {
        self.resolve_built::<T::Handle>()
    }

    /// Resolves `H` through this scope then each parent, or — if `H::Target` is a
    /// `Transient` — constructs a fresh instance.
    pub async fn resolve<H: Injectable>(self: &Arc<Self>) -> Option<H> {
        if let Some(handle) = self.resolve_built::<H>() {
            return Some(handle);
        }

        construct_transient::<H>(&self.registry, Some(Arc::clone(self))).await
    }

    /// Single concrete-or-primary-provider lookup across this scope and its parents.
    pub(crate) fn resolve_built<H: Injectable>(&self) -> Option<H> {
        if let Some(handle) = self.store.resolve_local::<H>() {
            return Some(handle);
        }

        self.parent.as_ref()?.resolve_built::<H>()
    }

    /// Qualifier-selected single provider across this scope and its parents.
    pub(crate) fn resolve_qualified_built<H: Injectable>(&self, qualifier: &str) -> Option<H> {
        if let Some(handle) = self.store.resolve_qualified_local::<H>(qualifier) {
            return Some(handle);
        }

        self.parent
            .as_ref()?
            .resolve_qualified_built::<H>(qualifier)
    }

    /// Every provider of the trait `H::Target` across this scope and its parents.
    pub(crate) fn collect_all_built<H: Injectable>(&self) -> Vec<H> {
        let mut all = self.store.collect_all_local::<H>();

        if let Some(parent) = &self.parent {
            all.extend(parent.collect_all_built::<H>());
        }

        all
    }

    /// Every provider of the trait `H::Target` keyed by qualifier across this scope
    /// and its parents (a closer scope wins a qualifier collision).
    pub(crate) fn collect_keyed_built<H: Injectable>(&self) -> HashMap<String, H> {
        let mut keyed = match &self.parent {
            Some(parent) => parent.collect_keyed_built::<H>(),
            None => HashMap::new(),
        };

        keyed.extend(self.store.collect_keyed_local::<H>());

        keyed
    }
}

/// Backwards-compatible alias: the root singleton store is a [`ScopeContainer`]
/// with no parent.
pub type ComponentContainer = ScopeContainer;

/// Registers every provider declared by the just-built concrete `concrete_id`,
/// aliasing its single instance under each trait it provides.
fn register_providers_for(
    cx: &mut ComponentConstructionContext,
    providers: &[ProviderDescriptor],
    concrete_id: TypeId,
) {
    for provider in providers
        .iter()
        .filter(|p| (p.concrete_ty.type_id)() == concrete_id)
    {
        cx.register_provider(provider);
    }
}

/// Computes a construction order for `components`, treating every `TypeId` in
/// `prebuilt` (seeded instances and longer-lived parent-scope components) as
/// already available. Config edges impose no ordering constraint — config values
/// are resolved through an external resolver, not constructed here.
pub fn topological_sort<'a>(
    components: &'a [ComponentDescriptor],
    prebuilt: &HashSet<TypeId>,
    providers: &[ProviderDescriptor],
) -> crate::Result<Vec<&'a ComponentDescriptor>> {
    trace!(total = components.len(), "starting topological sort");

    // trait TypeId -> the concrete TypeIds that provide it. A dependency on a
    // trait must wait for all of its providers to be built.
    let mut provider_concretes: HashMap<TypeId, Vec<TypeId>> = HashMap::new();

    for provider in providers {
        provider_concretes
            .entry((provider.trait_ty.type_id)())
            .or_default()
            .push((provider.concrete_ty.type_id)());
    }

    let mut result: Vec<&'a ComponentDescriptor> = Vec::new();
    let mut remaining: Vec<&'a ComponentDescriptor> = components.iter().collect();

    while !remaining.is_empty() {
        let before_len = remaining.len();

        remaining.retain(|descriptor| {
            let is_built = |type_id: TypeId| {
                prebuilt.contains(&type_id) || result.iter().any(|r| (r.ty.type_id)() == type_id)
            };

            let resolved = descriptor
                .dependencies()
                .iter()
                // `optional`/`dynamic`/`config` edges impose no build-ordering constraint.
                .filter(|dep| !dep.optional && !dep.dynamic && !dep.config)
                .all(|dep| {
                    dep_ready(
                        dep.cardinality,
                        (dep.ty.type_id)(),
                        &provider_concretes,
                        &is_built,
                    )
                });

            if resolved {
                trace!(component = %descriptor.name, "dependency order resolved");
                result.push(descriptor);
                false
            } else {
                true
            }
        });

        if remaining.len() == before_len {
            let stuck = remaining
                .iter()
                .map(|d| d.name)
                .collect::<Vec<_>>()
                .join(", ");

            error!(components = %stuck, "dependency cycle detected in component graph");

            return Err(Error::DependencyCycle(stuck));
        }
    }

    trace!(count = result.len(), "topological sort complete");

    Ok(result)
}

/// Whether a dependency's predecessors are all built. A trait edge waits for
/// every provider of that trait; a single concrete edge waits for that concrete;
/// a multi-valued edge with no providers is trivially ready (empty is valid).
fn dep_ready(
    cardinality: Cardinality,
    dep_type_id: TypeId,
    provider_concretes: &HashMap<TypeId, Vec<TypeId>>,
    is_built: &impl Fn(TypeId) -> bool,
) -> bool {
    if let Some(concretes) = provider_concretes.get(&dep_type_id) {
        return concretes.iter().all(|id| is_built(*id));
    }

    match cardinality {
        Cardinality::One => is_built(dep_type_id),
        Cardinality::Collection | Cardinality::Keyed => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use overseerd_core::TypeDescriptor;

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
}
