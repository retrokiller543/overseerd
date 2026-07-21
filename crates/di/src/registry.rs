use std::any::TypeId;
use std::collections::{HashMap, HashSet};

mod order;

use overseerd_core::{Cardinality, ResolutionMode, Scope, Singleton};

use crate::descriptors::{COMPONENTS, ComponentDescriptor, PROVIDERS, ProviderDescriptor};
use crate::error::Error;

/// Holds the component and provider *descriptors* of an application — declarations
/// only. Runtime instances live in the [`ScopeContainer`](crate::container::ScopeContainer).
///
/// This is the DI engine's own registry: it validates the component/provider graph
/// (ids, dependencies, scopes) and resolves the per-type descriptor set the container
/// builds from. Higher layers (services/RPC, config bindings) wrap it with their own
/// validation. Config edges (`#[config]`) are *skipped* here — they resolve against an
/// external resolver, validated by the config layer.
#[derive(Default, Debug, Clone)]
pub struct ComponentRegistry {
    pub components: Vec<ComponentDescriptor>,
    pub providers: Vec<ProviderDescriptor>,
}

impl ComponentRegistry {
    /// Collects every link-time-registered component and provider descriptor.
    pub fn collect() -> Self {
        let mut components: Vec<_> = COMPONENTS.iter().copied().collect();
        let mut providers: Vec<_> = PROVIDERS.iter().copied().collect();

        // linkme does not promise cross-platform iteration order. Stable discovery
        // keeps construction and lifecycle hook ordering equal on every target.
        components.sort_by_key(|component| component.id);
        providers.sort_by(|left, right| {
            (left.trait_ty.type_name)()
                .cmp((right.trait_ty.type_name)())
                .then_with(|| (left.concrete_ty.type_name)().cmp((right.concrete_ty.type_name)()))
                .then_with(|| left.qualifier.cmp(right.qualifier))
        });

        Self {
            components,
            providers,
        }
    }

    /// Collapses the registered descriptors to one per type. A manually-provided
    /// instance (an empty-factory descriptor) **overrides** an auto-constructed one
    /// for the same type. The per-type factory ambiguity check runs here via
    /// [`ComponentDescriptor::effective_factory`].
    pub fn resolved_components(&self) -> crate::Result<Vec<ComponentDescriptor>> {
        let mut chosen = Vec::new();
        let mut positions: HashMap<TypeId, usize> = HashMap::new();

        for component in &self.components {
            let type_id = component.ty.type_id;
            let new_manual = component.effective_factory()?.is_none();

            match positions.get(&type_id).copied() {
                None => {
                    positions.insert(type_id, chosen.len());
                    chosen.push(*component);
                }

                Some(position) => {
                    let existing = &chosen[position];
                    let existing_manual = existing.effective_factory()?.is_none();

                    if new_manual && !existing_manual {
                        // An override replaces the descriptor at its original position.
                        // Lifecycle hook ordering must not depend on HashMap iteration.
                        chosen[position] = *component;
                    } else if new_manual == existing_manual && existing.id != component.id {
                        return Err(Error::DuplicateComponentType(
                            (component.ty.type_name)().to_string(),
                        ));
                    }
                }
            }
        }

        Ok(chosen)
    }

    /// Validates the component graph: unique ids, satisfiable dependencies, and the
    /// captive-dependency scope rule.
    pub fn validate(&self) -> crate::Result<()> {
        let components = self.resolved_components()?;

        self.validate_component_ids(&components)?;
        self.validate_dependencies(&components)?;
        self.validate_provider_qualifiers(&components)?;
        self.validate_deferred_dependencies(&components)?;
        self.validate_fresh_dependencies(&components)?;
        self.provider_order(&components)?;
        self.validate_scopes(&components)?;

        Ok(())
    }

    /// Enforces the captive-dependency rule: a non-transient component may depend
    /// only on equal-or-longer-lived non-transient components. Checked against
    /// [`Scope::rank`], not by matching each label.
    pub fn validate_scopes(&self, components: &[ComponentDescriptor]) -> crate::Result<()> {
        let scope_of: HashMap<TypeId, &'static dyn Scope> =
            components.iter().map(|c| (c.ty.type_id, c.scope)).collect();

        for c in components {
            for dep in c.dependencies() {
                // Config edges resolve against external bindings; dynamic edges are
                // runtime-provided. Neither participates in the scope rule.
                if dep.dynamic || dep.config {
                    continue;
                }

                let dep_id = dep.ty.type_id;

                let dep_scopes: Vec<(&'static dyn Scope, &'static str)> =
                    match scope_of.get(&dep_id) {
                        Some(scope) => vec![(*scope, (dep.ty.type_name)())],

                        None => self
                            .providers
                            .iter()
                            .filter(|p| p.trait_ty.type_id == dep_id)
                            .filter_map(|p| {
                                scope_of
                                    .get(&p.concrete_ty.type_id)
                                    .map(|scope| (*scope, (p.concrete_ty.type_name)()))
                            })
                            .collect(),
                    };

                for (dep_scope, dep_name) in dep_scopes {
                    if dep.resolution != ResolutionMode::Fresh && !scope_allows(c.scope, dep_scope)
                    {
                        return Err(Error::ScopeViolation {
                            component: c.name.to_string(),
                            dependency: dep_name.to_string(),
                            component_scope: c.scope.name(),
                            dependency_scope: dep_scope.name(),
                        });
                    }
                }
            }
        }

        Ok(())
    }

    fn validate_component_ids(&self, components: &[ComponentDescriptor]) -> crate::Result<()> {
        let mut seen = HashSet::new();
        for c in components {
            if !seen.insert(c.id) {
                return Err(Error::DuplicateComponentId(c.id.to_string()));
            }
        }

        Ok(())
    }

    /// Validates that every non-config single dependency is satisfiable by a
    /// component or trait provider.
    pub fn validate_dependencies(&self, components: &[ComponentDescriptor]) -> crate::Result<()> {
        let available: HashSet<TypeId> = components.iter().map(|c| c.ty.type_id).collect();

        // Per trait: (total providers, primary providers).
        let mut provider_counts: HashMap<TypeId, (usize, usize)> = HashMap::new();

        for p in &self.providers {
            let counts = provider_counts.entry(p.trait_ty.type_id).or_default();
            counts.0 += 1;
            counts.1 += usize::from(p.primary);
        }

        for c in components {
            for dep in c.dependencies() {
                // Config edges are validated against bindings by the config layer.
                if dep.config {
                    continue;
                }

                let dep_id = dep.ty.type_id;
                let providers = provider_counts.get(&dep_id).copied();

                if let Some(qualifier) = dep.qualifier {
                    let found = dep.dynamic
                        || self
                            .providers
                            .iter()
                            .any(|p| p.trait_ty.type_id == dep_id && p.qualifier == qualifier);

                    if !found {
                        return Err(Error::MissingDependency {
                            component: c.name.to_string(),
                            type_name: format!(
                                "{} (qualifier `{qualifier}`)",
                                (dep.ty.type_name)()
                            ),
                        });
                    }

                    continue;
                }

                if dep.cardinality == Cardinality::One
                    && !dep.dynamic
                    && let Some((total, primary)) = providers
                    && total > 1
                    && primary != 1
                {
                    return Err(Error::AmbiguousProvider((dep.ty.type_name)().to_string()));
                }

                let must_exist =
                    dep.cardinality.requires_provider() && !dep.optional && !dep.dynamic;

                if must_exist && !available.contains(&dep_id) && providers.is_none() {
                    return Err(Error::MissingDependency {
                        component: c.name.to_string(),
                        type_name: (dep.ty.type_name)().to_string(),
                    });
                }
            }
        }

        Ok(())
    }

    /// Rejects duplicate `(trait, qualifier)` providers within one scope: a
    /// qualifier selects the first registered match, so two providers sharing a
    /// qualifier in the same scope would resolve by build order — descriptor-order
    /// dependent and effectively arbitrary. Distinct scopes may legitimately share
    /// a qualifier (the closer scope wins by design).
    pub fn validate_provider_qualifiers(
        &self,
        components: &[ComponentDescriptor],
    ) -> crate::Result<()> {
        let scope_of: HashMap<TypeId, &'static str> = components
            .iter()
            .map(|component| (component.ty.type_id, component.scope.name()))
            .collect();
        let mut seen: HashSet<(TypeId, &str, &str)> = HashSet::new();

        for provider in &self.providers {
            let Some(scope) = scope_of.get(&provider.concrete_ty.type_id).copied() else {
                continue;
            };

            if !seen.insert((provider.trait_ty.type_id, provider.qualifier, scope)) {
                return Err(Error::DuplicateProviderQualifier {
                    trait_name: (provider.trait_ty.type_name)().to_string(),
                    qualifier: provider.qualifier.to_string(),
                    scope: scope.to_string(),
                });
            }
        }

        Ok(())
    }

    /// Rejects deferred dependencies whose selected target is transient. Deferred
    /// handles retain only a weak reference, so their targets must be stored by a
    /// concrete scope after construction.
    pub fn validate_deferred_dependencies(
        &self,
        components: &[ComponentDescriptor],
    ) -> crate::Result<()> {
        let by_type: HashMap<TypeId, ComponentDescriptor> = components
            .iter()
            .map(|component| (component.ty.type_id, *component))
            .collect();

        for consumer in components {
            for dependency in consumer
                .dependencies()
                .into_iter()
                .filter(|dependency| dependency.resolution == ResolutionMode::Deferred)
            {
                let target = match by_type.get(&dependency.ty.type_id).copied() {
                    Some(target) => Some(target),
                    None => {
                        let matching: Vec<ProviderDescriptor> = self
                            .providers
                            .iter()
                            .filter(|provider| provider.trait_ty.type_id == dependency.ty.type_id)
                            .filter(|provider| {
                                dependency
                                    .qualifier
                                    .is_none_or(|qualifier| provider.qualifier == qualifier)
                            })
                            .copied()
                            .collect();
                        // Hydration resolves through scope stores, which never contain
                        // transient providers — so a transient can never be the hydrated
                        // target while a scoped alternative exists. Select among scoped
                        // providers with the runtime rule; only fall back to the full
                        // set to report a genuinely transient-only target.
                        let scoped: Vec<ProviderDescriptor> = matching
                            .iter()
                            .filter(|provider| {
                                by_type
                                    .get(&provider.concrete_ty.type_id)
                                    .is_none_or(|concrete| !concrete.scope.is_transient())
                            })
                            .copied()
                            .collect();
                        let candidates = if scoped.is_empty() {
                            &matching
                        } else {
                            &scoped
                        };
                        // Hydration searches the consumer's own scope, then each
                        // parent scope in turn, selecting per scope and stopping at
                        // the nearest scope that resolves — so candidates are
                        // grouped by scope rank and walked nearest-first rather
                        // than selected from the merged set. An ambiguous group
                        // falls through to the next parent scope, exactly like
                        // runtime `resolve_built`.
                        let consumer_rank = consumer.scope.rank();
                        let mut by_rank: std::collections::BTreeMap<u8, Vec<ProviderDescriptor>> =
                            std::collections::BTreeMap::new();

                        for provider in candidates {
                            let Some(concrete) = by_type.get(&provider.concrete_ty.type_id) else {
                                continue;
                            };

                            if concrete.scope.rank() >= consumer_rank {
                                by_rank
                                    .entry(concrete.scope.rank())
                                    .or_default()
                                    .push(*provider);
                            }
                        }

                        let select = |providers: &[ProviderDescriptor]| {
                            if dependency.qualifier.is_some() {
                                providers.first().copied()
                            } else {
                                crate::container::select_single_provider(providers)
                            }
                        };
                        let mut selected = None;
                        let mut saw_group = false;

                        for group in by_rank.values() {
                            saw_group = true;

                            if let Some(provider) = select(group) {
                                selected = Some(provider);
                                break;
                            }
                        }

                        match selected {
                            Some(provider) => by_type.get(&provider.concrete_ty.type_id).copied(),
                            // No visible candidates is a missing-dependency or
                            // scope-rule matter (reported elsewhere); candidates
                            // that exist but no scope in the chain can select are
                            // genuinely ambiguous.
                            None if !saw_group => None,
                            None => {
                                return Err(Error::AmbiguousProvider(
                                    (dependency.ty.type_name)().to_string(),
                                ));
                            }
                        }
                    }
                };

                if target.is_some_and(|target| target.scope.is_transient()) {
                    return Err(Error::DeferredTransientDependency {
                        component: consumer.name.to_string(),
                        dependency: (dependency.ty.type_name)().to_string(),
                    });
                }
            }
        }

        Ok(())
    }

    /// Validates that forced-fresh targets have factories and can resolve their eager
    /// dependencies from the consumer's scope.
    pub fn validate_fresh_dependencies(
        &self,
        components: &[ComponentDescriptor],
    ) -> crate::Result<()> {
        let by_type: HashMap<TypeId, ComponentDescriptor> = components
            .iter()
            .map(|component| (component.ty.type_id, *component))
            .collect();
        let scope_of: HashMap<TypeId, &'static dyn Scope> = components
            .iter()
            .map(|component| (component.ty.type_id, component.scope))
            .collect();

        for consumer in components {
            for dependency in consumer
                .dependencies()
                .into_iter()
                .filter(|dependency| dependency.resolution == ResolutionMode::Fresh)
            {
                let targets = self.fresh_targets(&dependency, &by_type);

                for target in targets {
                    // The consumer retains a fresh instance permanently, so the
                    // target's own scope must be visible from the consumer's scope —
                    // the same lifetime rule an eager edge follows. Collection shapes
                    // skip inaccessible providers at runtime, so only validate the
                    // accessible subset.
                    if !scope_allows(consumer.scope, target.scope) {
                        if dependency.cardinality == Cardinality::One {
                            return Err(Error::InvalidFreshDependency {
                                component: consumer.name.to_string(),
                                dependency: target.name.to_string(),
                            });
                        }

                        continue;
                    }

                    if target.effective_factory()?.is_none() {
                        return Err(Error::UnsupportedFreshFactory(target.name.to_string()));
                    }

                    for target_dependency in target.dependencies().into_iter().filter(|edge| {
                        edge.resolution == ResolutionMode::Eager && !edge.dynamic && !edge.config
                    }) {
                        let dependency_scopes: Vec<_> =
                            match scope_of.get(&target_dependency.ty.type_id) {
                                Some(scope) => vec![*scope],
                                None => self
                                    .providers
                                    .iter()
                                    .filter(|provider| {
                                        provider.trait_ty.type_id == target_dependency.ty.type_id
                                    })
                                    .filter_map(|provider| {
                                        scope_of.get(&provider.concrete_ty.type_id).copied()
                                    })
                                    .collect(),
                            };

                        if dependency_scopes
                            .iter()
                            .any(|scope| !scope_allows(consumer.scope, *scope))
                        {
                            return Err(Error::InvalidFreshDependency {
                                component: consumer.name.to_string(),
                                dependency: format!(
                                    "{} -> {}",
                                    target.name,
                                    (target_dependency.ty.type_name)()
                                ),
                            });
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn fresh_targets(
        &self,
        dependency: &overseerd_core::DependencyDescriptor,
        by_type: &HashMap<TypeId, ComponentDescriptor>,
    ) -> Vec<ComponentDescriptor> {
        if let Some(component) = by_type.get(&dependency.ty.type_id) {
            return vec![*component];
        }

        self.providers
            .iter()
            .filter(|provider| provider.trait_ty.type_id == dependency.ty.type_id)
            .filter(|provider| {
                dependency
                    .qualifier
                    .is_none_or(|qualifier| provider.qualifier == qualifier)
            })
            .filter_map(|provider| by_type.get(&provider.concrete_ty.type_id).copied())
            .collect()
    }
    /// Validates and topologically orders all providers independently per trait.
    pub fn provider_order(
        &self,
        components: &[ComponentDescriptor],
    ) -> crate::Result<HashMap<TypeId, HashMap<TypeId, usize>>> {
        order::build(components, &self.providers)
    }
}

/// Whether a `consumer`-scoped component may hold a `dependency`-scoped one.
fn scope_allows(consumer: &dyn Scope, dependency: &dyn Scope) -> bool {
    if dependency.is_transient() {
        return true;
    }

    if consumer.is_transient() {
        return dependency.rank() == Singleton.rank();
    }

    dependency.rank() >= consumer.rank()
}

#[cfg(test)]
mod tests {
    use std::{future::Future, pin::Pin};

    use super::*;
    use crate::descriptors::{
        BoxedComponent, ComponentConstructionContext, ComponentDescriptor,
        ComponentFactoryDescriptor,
    };
    use overseerd_core::{Cardinality, DependencyDescriptor, Transient, TypeDescriptor};

    /// Local stand-in intermediate scopes (the captive rule only cares about rank
    /// ordering): `Connection` outranks `Request`, both between singleton and transient.
    /// They are defined here rather than imported so the DI engine stays unaware of any
    /// protocol's concrete scopes.
    struct Connection;
    struct Request;

    impl Scope for Connection {
        fn rank(&self) -> u8 {
            2
        }

        fn name(&self) -> &'static str {
            "Connection"
        }
    }

    impl Scope for Request {
        fn rank(&self) -> u8 {
            1
        }

        fn name(&self) -> &'static str {
            "Request"
        }
    }

    fn fake_factory<'a>(
        _: &'a mut ComponentConstructionContext,
    ) -> Pin<Box<dyn Future<Output = crate::Result<BoxedComponent>> + Send + 'a>> {
        Box::pin(async { todo!() })
    }

    macro_rules! scoped {
        ($name:expr, $scope:expr, $dep:expr, $ty:expr $(,)?) => {{
            fn deps() -> ::std::vec::Vec<DependencyDescriptor> {
                $dep.to_vec()
            }

            static FACTORIES: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
                construct: fake_factory,
                dependencies: deps,
                default: false,
            }];

            fn factories() -> &'static [ComponentFactoryDescriptor] {
                &FACTORIES
            }

            ComponentDescriptor {
                id: $name,
                name: $name,
                ty: $ty,
                scope: $scope,
                factories,
                hooks: ::overseerd_hooks::no_hooks,
            }
        }};
    }

    fn pg_pool_deps() -> Vec<DependencyDescriptor> {
        Vec::new()
    }

    static PG_POOL_FACTORIES: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
        construct: fake_factory,
        dependencies: pg_pool_deps,
        default: false,
    }];

    fn pg_pool_factories() -> &'static [ComponentFactoryDescriptor] {
        &PG_POOL_FACTORIES
    }

    static PG_POOL: ComponentDescriptor = ComponentDescriptor {
        id: "pg_pool",
        name: "PgPool",
        ty: TypeDescriptor::of::<u16>("PgPool"),
        scope: &Singleton,
        factories: pg_pool_factories,
        hooks: overseerd_hooks::no_hooks,
    };

    fn backup_repo_deps() -> Vec<DependencyDescriptor> {
        vec![DependencyDescriptor {
            name: "PgPool",
            ty: TypeDescriptor::of::<u16>("PgPool"),
            cardinality: Cardinality::One,
            optional: false,
            dynamic: false,
            qualifier: None,
            config: false,
            resolution: ResolutionMode::Eager,
        }]
    }

    static BACKUP_REPO_FACTORIES: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
        construct: fake_factory,
        dependencies: backup_repo_deps,
        default: false,
    }];

    fn backup_repo_factories() -> &'static [ComponentFactoryDescriptor] {
        &BACKUP_REPO_FACTORIES
    }

    static BACKUP_REPO: ComponentDescriptor = ComponentDescriptor {
        id: "backup_repo",
        name: "BackupRepository",
        ty: TypeDescriptor::of::<u8>("BackupRepository"),
        scope: &Singleton,
        factories: backup_repo_factories,
        hooks: overseerd_hooks::no_hooks,
    };

    #[test]
    fn validate_passes_with_fulfilled_dependencies() {
        let registry = ComponentRegistry {
            components: vec![BACKUP_REPO, PG_POOL],
            ..Default::default()
        };

        assert!(registry.validate().is_ok());
    }

    #[test]
    fn validate_detects_duplicate_component_ids() {
        let registry = ComponentRegistry {
            components: vec![BACKUP_REPO, BACKUP_REPO],
            ..Default::default()
        };

        assert!(registry.validate().is_err());
    }

    #[test]
    fn validate_detects_missing_dependency() {
        let registry = ComponentRegistry {
            components: vec![BACKUP_REPO],
            ..Default::default()
        };

        assert!(registry.validate().is_err());
    }

    #[test]
    fn validate_accepts_manual_component_descriptor() {
        let without = ComponentRegistry {
            components: vec![BACKUP_REPO],
            ..Default::default()
        };

        assert!(without.validate().is_err());

        let manual = ComponentDescriptor::manual(
            "pg_pool_manual",
            "PgPool",
            TypeDescriptor::of::<u16>("PgPool"),
            &Singleton,
        );
        let with = ComponentRegistry {
            components: vec![BACKUP_REPO, manual],
            ..Default::default()
        };

        assert!(with.validate().is_ok());
    }

    #[test]
    fn resolved_components_preserve_registration_order_through_manual_override() {
        let manual = ComponentDescriptor::manual(
            "pg_pool_manual",
            "PgPool",
            TypeDescriptor::of::<u16>("PgPool"),
            &Singleton,
        );
        let registry = ComponentRegistry {
            components: vec![PG_POOL, BACKUP_REPO, manual],
            ..Default::default()
        };

        let resolved = registry.resolved_components().expect("resolve components");
        let ids: Vec<_> = resolved.iter().map(|component| component.id).collect();

        assert_eq!(ids, ["pg_pool_manual", "backup_repo"]);
    }

    // --- Scope validation --------------------------------------------------
    //
    // Stand-in types per scope: i16 = singleton, i32 = connection, i64 = request.

    static SINGLETON_DEP_ON_REQUEST: [DependencyDescriptor; 1] = [DependencyDescriptor {
        name: "ReqComp",
        ty: TypeDescriptor::of::<i64>("ReqComp"),
        cardinality: Cardinality::One,
        optional: false,
        dynamic: false,
        qualifier: None,
        config: false,
        resolution: ResolutionMode::Eager,
    }];

    static REQUEST_DEP_ON_CONNECTION: [DependencyDescriptor; 1] = [DependencyDescriptor {
        name: "ConnComp",
        ty: TypeDescriptor::of::<i32>("ConnComp"),
        cardinality: Cardinality::One,
        optional: false,
        dynamic: false,
        qualifier: None,
        config: false,
        resolution: ResolutionMode::Eager,
    }];

    static SINGLETON_DEFERRED_TRANSIENT: [DependencyDescriptor; 1] = [DependencyDescriptor {
        name: "TransientDeferred",
        ty: TypeDescriptor::of::<u32>("TransientDeferred"),
        cardinality: Cardinality::One,
        optional: false,
        dynamic: false,
        qualifier: None,
        config: false,
        resolution: ResolutionMode::Deferred,
    }];

    static SINGLETON_FRESH_REQUEST: [DependencyDescriptor; 1] = [DependencyDescriptor {
        name: "RequestFresh",
        ty: TypeDescriptor::of::<u128>("RequestFresh"),
        cardinality: Cardinality::One,
        optional: false,
        dynamic: false,
        qualifier: None,
        config: false,
        resolution: ResolutionMode::Fresh,
    }];

    #[test]
    fn validate_rejects_fresh_target_with_shorter_lived_scope() {
        let request = scoped!(
            "RequestFresh",
            &Request,
            &[],
            TypeDescriptor::of::<u128>("RequestFresh"),
        );
        let singleton = scoped!(
            "FreshConsumer",
            &Singleton,
            &SINGLETON_FRESH_REQUEST,
            TypeDescriptor::of::<i8>("FreshConsumer"),
        );
        let registry = ComponentRegistry {
            components: vec![singleton, request],
            ..Default::default()
        };

        assert!(matches!(
            registry.validate(),
            Err(Error::InvalidFreshDependency { .. })
        ));
    }

    #[test]
    fn validate_allows_lazy_dependency_on_transient_target() {
        static SINGLETON_LAZY_TRANSIENT: [DependencyDescriptor; 1] = [DependencyDescriptor {
            name: "TransientLazy",
            ty: TypeDescriptor::of::<u32>("TransientLazy"),
            cardinality: Cardinality::One,
            optional: false,
            dynamic: false,
            qualifier: None,
            config: false,
            resolution: ResolutionMode::Lazy,
        }];
        let transient = scoped!(
            "TransientLazy",
            &Transient,
            &[],
            TypeDescriptor::of::<u32>("TransientLazy"),
        );
        let singleton = scoped!(
            "LazyConsumer",
            &Singleton,
            &SINGLETON_LAZY_TRANSIENT,
            TypeDescriptor::of::<u8>("LazyConsumer"),
        );
        let registry = ComponentRegistry {
            components: vec![singleton, transient],
            ..Default::default()
        };

        assert!(registry.validate().is_ok());
    }

    fn unreachable_erase(_: &BoxedComponent) -> BoxedComponent {
        unreachable!("validation never erases providers")
    }

    fn trait_provider(
        concrete: TypeDescriptor,
        qualifier: &'static str,
        primary: bool,
    ) -> ProviderDescriptor {
        ProviderDescriptor {
            trait_ty: TypeDescriptor::of::<dyn Send>("SharedTrait"),
            concrete_ty: concrete,
            qualifier,
            primary,
            priority: 0,
            ordering: &[],
            erase: unreachable_erase,
        }
    }

    #[test]
    fn validate_rejects_duplicate_provider_qualifier_in_one_scope() {
        let first = scoped!(
            "FirstQ",
            &Singleton,
            &[],
            TypeDescriptor::of::<u8>("FirstQ")
        );
        let second = scoped!(
            "SecondQ",
            &Singleton,
            &[],
            TypeDescriptor::of::<u16>("SecondQ")
        );
        let registry = ComponentRegistry {
            components: vec![first, second],
            providers: vec![
                trait_provider(TypeDescriptor::of::<u8>("FirstQ"), "same", false),
                trait_provider(TypeDescriptor::of::<u16>("SecondQ"), "same", false),
            ],
        };

        assert!(matches!(
            registry.validate(),
            Err(Error::DuplicateProviderQualifier { .. })
        ));
    }

    #[test]
    fn validate_allows_duplicate_provider_qualifier_across_scopes() {
        let first = scoped!(
            "FirstQ",
            &Singleton,
            &[],
            TypeDescriptor::of::<u8>("FirstQ")
        );
        let second = scoped!(
            "SecondQ",
            &Request,
            &[],
            TypeDescriptor::of::<u16>("SecondQ")
        );
        let registry = ComponentRegistry {
            components: vec![first, second],
            providers: vec![
                trait_provider(TypeDescriptor::of::<u8>("FirstQ"), "same", false),
                trait_provider(TypeDescriptor::of::<u16>("SecondQ"), "same", false),
            ],
        };

        assert!(registry.validate().is_ok());
    }

    #[test]
    fn validate_allows_deferred_trait_with_scoped_provider_beside_transient_primary() {
        static SINGLETON_DEFERRED_SHARED: [DependencyDescriptor; 1] = [DependencyDescriptor {
            name: "SharedTrait",
            ty: TypeDescriptor::of::<dyn Send>("SharedTrait"),
            cardinality: Cardinality::One,
            optional: false,
            dynamic: false,
            qualifier: None,
            config: false,
            resolution: ResolutionMode::Deferred,
        }];
        let consumer = scoped!(
            "DeferredTraitConsumer",
            &Singleton,
            &SINGLETON_DEFERRED_SHARED,
            TypeDescriptor::of::<u64>("DeferredTraitConsumer"),
        );
        let scoped_provider = scoped!(
            "ScopedProvider",
            &Singleton,
            &[],
            TypeDescriptor::of::<u8>("ScopedProvider"),
        );
        let transient_provider = scoped!(
            "TransientProvider",
            &Transient,
            &[],
            TypeDescriptor::of::<u16>("TransientProvider"),
        );
        let registry = ComponentRegistry {
            components: vec![consumer, scoped_provider, transient_provider],
            providers: vec![
                trait_provider(
                    TypeDescriptor::of::<u16>("TransientProvider"),
                    "transient",
                    true,
                ),
                trait_provider(TypeDescriptor::of::<u8>("ScopedProvider"), "scoped", false),
            ],
        };

        // Hydration resolves through scope stores, which never hold transients,
        // so the scoped provider — not the transient global primary — is the
        // validated target.
        assert!(registry.validate().is_ok());
    }

    static SINGLETON_DEFERRED_SHARED: [DependencyDescriptor; 1] = [DependencyDescriptor {
        name: "SharedTrait",
        ty: TypeDescriptor::of::<dyn Send>("SharedTrait"),
        cardinality: Cardinality::One,
        optional: false,
        dynamic: false,
        qualifier: None,
        config: false,
        resolution: ResolutionMode::Deferred,
    }];

    #[test]
    fn validate_rejects_ambiguous_deferred_candidates() {
        let consumer = scoped!(
            "AmbiguousDeferredConsumer",
            &Singleton,
            &SINGLETON_DEFERRED_SHARED,
            TypeDescriptor::of::<u64>("AmbiguousDeferredConsumer"),
        );
        let first = scoped!(
            "FirstScoped",
            &Singleton,
            &[],
            TypeDescriptor::of::<u8>("FirstScoped")
        );
        let second = scoped!(
            "SecondScoped",
            &Singleton,
            &[],
            TypeDescriptor::of::<u16>("SecondScoped"),
        );
        let transient = scoped!(
            "TransientPrimary",
            &Transient,
            &[],
            TypeDescriptor::of::<u32>("TransientPrimary"),
        );
        let registry = ComponentRegistry {
            components: vec![consumer, first, second, transient],
            providers: vec![
                trait_provider(TypeDescriptor::of::<u32>("TransientPrimary"), "t", true),
                trait_provider(TypeDescriptor::of::<u8>("FirstScoped"), "a", false),
                trait_provider(TypeDescriptor::of::<u16>("SecondScoped"), "b", false),
            ],
        };

        // The transient primary passes the global primary-count check, but after
        // transient filtering the scoped set is ambiguous with no parent fallback
        // left — hydration could never select deterministically.
        assert!(matches!(
            registry.validate(),
            Err(Error::AmbiguousProvider(_))
        ));
    }

    #[test]
    fn validate_selects_deferred_candidates_scope_locally() {
        static REQUEST_DEFERRED_SHARED: [DependencyDescriptor; 1] = [DependencyDescriptor {
            name: "SharedTrait",
            ty: TypeDescriptor::of::<dyn Send>("SharedTrait"),
            cardinality: Cardinality::One,
            optional: false,
            dynamic: false,
            qualifier: None,
            config: false,
            resolution: ResolutionMode::Deferred,
        }];
        let consumer = scoped!(
            "RequestDeferredConsumer",
            &Request,
            &REQUEST_DEFERRED_SHARED,
            TypeDescriptor::of::<u64>("RequestDeferredConsumer"),
        );
        let local = scoped!(
            "LocalProvider",
            &Request,
            &[],
            TypeDescriptor::of::<u8>("LocalProvider"),
        );
        let parent = scoped!(
            "ParentProvider",
            &Singleton,
            &[],
            TypeDescriptor::of::<u16>("ParentProvider"),
        );
        let transient = scoped!(
            "TransientPrimary",
            &Transient,
            &[],
            TypeDescriptor::of::<u32>("TransientPrimary"),
        );
        let registry = ComponentRegistry {
            components: vec![consumer, local, parent, transient],
            providers: vec![
                trait_provider(TypeDescriptor::of::<u32>("TransientPrimary"), "t", true),
                trait_provider(TypeDescriptor::of::<u8>("LocalProvider"), "local", false),
                trait_provider(TypeDescriptor::of::<u16>("ParentProvider"), "parent", false),
            ],
        };

        // The consumer's own scope has an unambiguous local provider, so the
        // cross-scope candidate set is not ambiguous for hydration.
        assert!(registry.validate().is_ok());
    }

    #[test]
    fn validate_walks_deferred_candidates_in_scope_chain_order() {
        static REQUEST_DEFERRED_CHAIN: [DependencyDescriptor; 1] = [DependencyDescriptor {
            name: "SharedTrait",
            ty: TypeDescriptor::of::<dyn Send>("SharedTrait"),
            cardinality: Cardinality::One,
            optional: false,
            dynamic: false,
            qualifier: None,
            config: false,
            resolution: ResolutionMode::Deferred,
        }];
        let consumer = scoped!(
            "ChainConsumer",
            &Request,
            &REQUEST_DEFERRED_CHAIN,
            TypeDescriptor::of::<u64>("ChainConsumer"),
        );
        let connection = scoped!(
            "ConnectionProvider",
            &Connection,
            &[],
            TypeDescriptor::of::<u8>("ConnectionProvider"),
        );
        let singleton = scoped!(
            "SingletonProvider",
            &Singleton,
            &[],
            TypeDescriptor::of::<u16>("SingletonProvider"),
        );
        let transient = scoped!(
            "TransientPrimary",
            &Transient,
            &[],
            TypeDescriptor::of::<u32>("TransientPrimary"),
        );
        let registry = ComponentRegistry {
            components: vec![consumer, connection, singleton, transient],
            providers: vec![
                trait_provider(TypeDescriptor::of::<u32>("TransientPrimary"), "t", true),
                trait_provider(
                    TypeDescriptor::of::<u8>("ConnectionProvider"),
                    "conn",
                    false,
                ),
                trait_provider(
                    TypeDescriptor::of::<u16>("SingletonProvider"),
                    "root",
                    false,
                ),
            ],
        };

        // The merged non-transient set is ambiguous, but runtime hydration walks
        // scope by scope and the connection scope selects its sole provider —
        // so this is a valid registry, not an ambiguous one.
        assert!(registry.validate().is_ok());
    }

    #[test]
    fn validate_rejects_singleton_depending_on_request() {
        let request = scoped!(
            "ReqComp",
            &Request,
            &[],
            TypeDescriptor::of::<i64>("ReqComp"),
        );
        let singleton = scoped!(
            "RootComp",
            &Singleton,
            &SINGLETON_DEP_ON_REQUEST,
            TypeDescriptor::of::<i16>("RootComp"),
        );

        let registry = ComponentRegistry {
            components: vec![singleton, request],
            ..Default::default()
        };

        assert!(matches!(
            registry.validate(),
            Err(Error::ScopeViolation { .. })
        ));
    }

    #[test]
    fn validate_allows_request_depending_on_connection() {
        let connection = scoped!(
            "ConnComp",
            &Connection,
            &[],
            TypeDescriptor::of::<i32>("ConnComp"),
        );
        let request = scoped!(
            "ReqComp",
            &Request,
            &REQUEST_DEP_ON_CONNECTION,
            TypeDescriptor::of::<i64>("ReqComp"),
        );

        let registry = ComponentRegistry {
            components: vec![connection, request],
            ..Default::default()
        };

        assert!(registry.validate_scopes(&registry.components).is_ok());
    }

    #[test]
    fn validate_rejects_transient_depending_on_connection() {
        let connection = scoped!(
            "ConnComp",
            &Connection,
            &[],
            TypeDescriptor::of::<i32>("ConnComp"),
        );
        let transient = scoped!(
            "TransComp",
            &Transient,
            &REQUEST_DEP_ON_CONNECTION,
            TypeDescriptor::of::<i64>("TransComp"),
        );

        let registry = ComponentRegistry {
            components: vec![connection, transient],
            ..Default::default()
        };

        assert!(matches!(
            registry.validate_scopes(&registry.components),
            Err(Error::ScopeViolation { .. })
        ));
    }

    #[test]
    fn validate_rejects_deferred_transient_target() {
        let transient = scoped!(
            "TransientDeferred",
            &Transient,
            &[],
            TypeDescriptor::of::<u32>("TransientDeferred"),
        );
        let singleton = scoped!(
            "DeferredConsumer",
            &Singleton,
            &SINGLETON_DEFERRED_TRANSIENT,
            TypeDescriptor::of::<u64>("DeferredConsumer"),
        );
        let registry = ComponentRegistry {
            components: vec![singleton, transient],
            ..Default::default()
        };

        assert!(matches!(
            registry.validate(),
            Err(Error::DeferredTransientDependency { .. })
        ));
    }
}
