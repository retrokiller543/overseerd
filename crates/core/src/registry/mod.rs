use crate::{
    DependencyDescriptor, ParameterDescriptor, RpcDescriptor,
    config::ConfigBinding,
    descriptors::{
        COMPONENTS, CONFIG_BINDINGS, Cardinality, ComponentDescriptor, ComponentScope, PROVIDERS,
        ProviderDescriptor, SERVICES, ServiceDescriptor,
    },
    error::Error,
};
use std::any::TypeId;
use std::collections::HashMap;
use std::fmt::Write;
use std::{collections::HashSet, fmt};

/// Holds the component, service, and RPC *descriptors* for a daemon —
/// declarations only. Runtime instances live in the `ComponentContainer`.
///
/// Component descriptors are owned (a flat `Vec`, since the descriptor is
/// `Copy`) so that descriptors synthesized at runtime for manually-provided
/// instances sit alongside link-time-collected ones. A component whose
/// `factory` is `None` is provided as an instance rather than constructed.
#[derive(Default, Debug)]
pub struct DescriptorRegistry {
    pub components: Vec<ComponentDescriptor>,
    pub services: Vec<ServiceDescriptor>,
    pub providers: Vec<ProviderDescriptor>,
    /// Config bindings (a config type bound to a property path). Populated from the
    /// auto-discovered [`CONFIG_BINDINGS`] slice and from explicit builder bindings.
    pub config_bindings: Vec<ConfigBinding>,
}

/// A service header with its RPCs, drawn from the service's own per-service slice.
pub struct ResolvedService {
    pub descriptor: ServiceDescriptor,
    pub rpcs: Vec<&'static RpcDescriptor>,
}

impl DescriptorRegistry {
    /// Collects every link-time-registered descriptor into a DescriptorRegistry.
    pub fn collect() -> Self {
        Self {
            components: COMPONENTS.iter().copied().collect(),
            services: SERVICES.iter().copied().collect(),
            providers: PROVIDERS.iter().copied().collect(),
            config_bindings: CONFIG_BINDINGS.iter().map(|d| d.to_binding()).collect(),
        }
    }

    /// Assembles each service header with the RPCs it owns. Services are deduped by
    /// type, so registering a service both manually and via auto-discovery yields a
    /// single resolved service (its RPCs come from its own slice, never doubled).
    pub fn resolved_services(&self) -> Vec<ResolvedService> {
        let mut seen = HashSet::new();

        self.services
            .iter()
            .filter(|descriptor| seen.insert((descriptor.ty.type_id)()))
            .map(|descriptor| {
                let rpcs = (descriptor.rpcs)()
                    .iter()
                    .flat_map(|group| group.rpcs.iter())
                    .collect();

                ResolvedService {
                    descriptor: *descriptor,
                    rpcs,
                }
            })
            .collect()
    }

    /// Collapses the registered descriptors to one per type. There is normally a
    /// single `ComponentDescriptor` per type (the macro-emitted one, possibly
    /// collected both via `auto_discover` and by-type registration — identical, so
    /// deduped). A manually-provided instance (`with_component`, an empty-factory
    /// descriptor) **overrides** an auto-constructed one for the same type, so an
    /// app can supply a hand-built instance and still auto-discover the rest. The
    /// per-type factory ambiguity check (an `#[init]` *and* a `factory = ..`) runs
    /// here via [`ComponentDescriptor::effective_factory`].
    pub fn resolved_components(&self) -> crate::Result<Vec<ComponentDescriptor>> {
        let mut chosen: HashMap<TypeId, ComponentDescriptor> = HashMap::new();

        for component in &self.components {
            let type_id = (component.ty.type_id)();
            let new_manual = component.effective_factory()?.is_none();

            match chosen.get(&type_id) {
                None => {
                    chosen.insert(type_id, *component);
                }

                Some(existing) => {
                    let existing_manual = existing.effective_factory()?.is_none();

                    // A provided instance (manual) overrides auto-construction.
                    if new_manual && !existing_manual {
                        chosen.insert(type_id, *component);
                    } else if new_manual == existing_manual && existing.id != component.id {
                        // Two distinct constructable (or two distinct manual)
                        // descriptors for one type — genuinely ambiguous.
                        return Err(Error::DuplicateComponentType(
                            (component.ty.type_name)().to_string(),
                        ));
                    }
                }
            }
        }

        Ok(chosen.into_values().collect())
    }

    /// Validates structural consistency of the registry.
    ///
    /// Manually-provided components are first-class descriptors here (with a
    /// `None` factory), so dependency checking needs no external set.
    pub fn validate(&self) -> crate::Result<()> {
        let components = self.resolved_components()?;

        self.validate_component_ids(&components)?;
        self.validate_services()?;
        self.validate_dependencies(&components)?;
        self.validate_scopes(&components)?;
        self.validate_configs(&components)?;

        Ok(())
    }

    /// Validates config edges against the registered bindings: a `#[config("path")]`
    /// edge must have a binding of its type at that path, and a `#[config]` shorthand
    /// edge must have exactly one binding of its type.
    fn validate_configs(&self, components: &[ComponentDescriptor]) -> crate::Result<()> {
        let mut bound: HashMap<TypeId, Vec<&str>> = HashMap::new();

        for binding in &self.config_bindings {
            bound
                .entry((binding.ty.type_id)())
                .or_default()
                .push(&binding.path);
        }

        for c in components {
            for dep in c.dependencies().iter().filter(|dep| dep.config) {
                let dep_id = (dep.ty.type_id)();
                let paths = bound.get(&dep_id);

                match dep.qualifier {
                    Some(path) => {
                        let found = paths.is_some_and(|ps| ps.contains(&path));

                        if !found {
                            return Err(Error::MissingConfig {
                                component: c.name.to_string(),
                                type_name: (dep.ty.type_name)().to_string(),
                                path: path.to_string(),
                            });
                        }
                    }

                    None => {
                        let bound_paths = paths.cloned().unwrap_or_default();

                        if bound_paths.len() != 1 {
                            return Err(Error::AmbiguousConfig {
                                component: c.name.to_string(),
                                type_name: (dep.ty.type_name)().to_string(),
                                count: bound_paths.len(),
                                paths: bound_paths.join(", "),
                            });
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Enforces the captive-dependency rule: a non-transient component may depend
    /// only on equal-or-longer-lived non-transient components. A `Transient`
    /// dependency is always allowed (it is rebuilt inside its consumer, never
    /// shared); a `Transient` *component* may depend only on singletons in v1, so
    /// it is safe to construct in any scope.
    ///
    /// The rule is checked against [`ComponentScope::rank`], not by matching each
    /// variant, so a future user-defined scope slots in at its own rank.
    fn validate_scopes(&self, components: &[ComponentDescriptor]) -> crate::Result<()> {
        let scope_of: HashMap<TypeId, ComponentScope> = components
            .iter()
            .map(|c| ((c.ty.type_id)(), c.scope))
            .collect();

        for c in components {
            for dep in c.dependencies() {
                // Config edges resolve against singleton bindings (validated in
                // `validate_configs`), so they never violate the scope rule.
                if dep.dynamic || dep.config {
                    continue;
                }

                let dep_id = (dep.ty.type_id)();

                // A concrete edge resolves to that type's scope; a trait edge to the
                // scope of each component providing the trait.
                let dep_scopes: Vec<(ComponentScope, &'static str)> = match scope_of.get(&dep_id) {
                    Some(scope) => vec![(*scope, (dep.ty.type_name)())],

                    None => self
                        .providers
                        .iter()
                        .filter(|p| (p.trait_ty.type_id)() == dep_id)
                        .filter_map(|p| {
                            scope_of
                                .get(&(p.concrete_ty.type_id)())
                                .map(|scope| (*scope, (p.concrete_ty.type_name)()))
                        })
                        .collect(),
                };

                for (dep_scope, dep_name) in dep_scopes {
                    if !scope_allows(c.scope, dep_scope) {
                        return Err(Error::ScopeViolation {
                            component: c.name.to_string(),
                            dependency: dep_name.to_string(),
                            component_scope: c.scope,
                            dependency_scope: dep_scope,
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

    fn validate_services(&self) -> crate::Result<()> {
        let mut seen_ids = HashSet::new();
        let mut seen_paths: HashSet<String> = HashSet::new();

        for service in self.resolved_services() {
            let s = service.descriptor;

            if !seen_ids.insert(s.id) {
                return Err(Error::DuplicateServiceId(s.id.to_string()));
            }

            if service.rpcs.is_empty() {
                return Err(Error::EmptyService(s.name.to_string()));
            }

            let mut seen_rpc_names = HashSet::new();
            for rpc in &service.rpcs {
                if !seen_rpc_names.insert(rpc.name) {
                    return Err(Error::DuplicateRpcName {
                        service: s.name.to_string(),
                        rpc: rpc.name.to_string(),
                    });
                }

                let path = format!("{}.{}", s.name, rpc.name);
                if !seen_paths.insert(path.clone()) {
                    return Err(Error::DuplicateRpcPath(path));
                }
            }
        }

        Ok(())
    }

    fn validate_dependencies(&self, components: &[ComponentDescriptor]) -> crate::Result<()> {
        let available: HashSet<TypeId> = components.iter().map(|c| (c.ty.type_id)()).collect();

        // Per trait: (total providers, primary providers).
        let mut provider_counts: HashMap<TypeId, (usize, usize)> = HashMap::new();

        for p in &self.providers {
            let counts = provider_counts.entry((p.trait_ty.type_id)()).or_default();
            counts.0 += 1;
            counts.1 += usize::from(p.primary);
        }

        for c in components {
            for dep in c.dependencies() {
                // Config edges are validated against bindings in `validate_configs`,
                // not against the component/provider graph.
                if dep.config {
                    continue;
                }

                let dep_id = (dep.ty.type_id)();
                let providers = provider_counts.get(&dep_id).copied();

                // A `#[qualifier]` edge names its provider explicitly: it must
                // exist, but it is never ambiguous even with several providers.
                if let Some(qualifier) = dep.qualifier {
                    let found = dep.dynamic
                        || self
                            .providers
                            .iter()
                            .any(|p| (p.trait_ty.type_id)() == dep_id && p.qualifier == qualifier);

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

                // An unqualified single `Arc<dyn Trait>` edge with several
                // providers needs a unique `#[primary]` to disambiguate — this is
                // distinct from "missing".
                if dep.cardinality == Cardinality::One
                    && !dep.dynamic
                    && let Some((total, primary)) = providers
                    && total > 1
                    && primary != 1
                {
                    return Err(Error::AmbiguousProvider((dep.ty.type_name)().to_string()));
                }

                // Multi-valued edges (Collection/Keyed) accept zero providers;
                // `optional` tolerates absence; `dynamic` providers are supplied
                // at runtime and so are exempt from static validation.
                let must_exist =
                    dep.cardinality.requires_provider() && !dep.optional && !dep.dynamic;

                // A single edge is satisfied by a concrete component of that type
                // or by at least one provider of that trait.
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

    fn write_components(&self, f: &mut impl Write) -> fmt::Result {
        let components = self
            .resolved_components()
            .unwrap_or_else(|_| self.components.clone());

        writeln!(f, "Components:")?;
        for c in &components {
            writeln!(f, "  {}", c.name)?;

            let deps = c.dependencies();

            if !deps.is_empty() {
                Self::write_dependency(f, deps.iter())?
            }
        }

        Ok(())
    }

    fn write_dependency<'a>(
        f: &mut impl Write,
        deps: impl Iterator<Item = &'a DependencyDescriptor>,
    ) -> fmt::Result {
        write!(f, "    depends on:")?;
        for (i, dep) in deps.enumerate() {
            if i > 0 {
                write!(f, ",")?;
            }

            write!(f, " {}", dep.name)?;
        }

        writeln!(f)?;

        Ok(())
    }

    fn write_services(&self, f: &mut impl Write) -> fmt::Result {
        writeln!(f, "Services:")?;
        for service in self.resolved_services() {
            let s = service.descriptor;

            match s.version {
                Some(v) => writeln!(f, "  {} (v{})", s.name, v)?,
                None => writeln!(f, "  {}", s.name)?,
            }

            Self::write_rpcs(f, service.rpcs.iter().copied())?;
        }

        Ok(())
    }

    fn write_rpcs<'a>(
        f: &mut impl Write,
        rpcs: impl Iterator<Item = &'a RpcDescriptor>,
    ) -> fmt::Result {
        for rpc in rpcs {
            write!(f, "    rpc {}(", rpc.name)?;
            Self::write_parameters(f, rpc.parameters.iter())?;
            writeln!(f, ") -> {}", rpc.output.name)?;
        }

        Ok(())
    }

    fn write_parameters<'a>(
        f: &mut impl Write,
        params: impl Iterator<Item = &'a ParameterDescriptor>,
    ) -> fmt::Result {
        for (i, param) in params.enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }

            write!(f, "{}", param.ty.name)?;
        }

        Ok(())
    }
}

/// Whether a `consumer`-scoped component may hold a `dependency`-scoped one. See
/// [`DescriptorRegistry::validate_scopes`].
fn scope_allows(consumer: ComponentScope, dependency: ComponentScope) -> bool {
    if dependency.is_transient() {
        return true;
    }

    if consumer.is_transient() {
        return dependency == ComponentScope::Singleton;
    }

    dependency.rank() >= consumer.rank()
}

impl fmt::Display for DescriptorRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_components(f)?;
        writeln!(f)?;
        self.write_services(f)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{future::Future, pin::Pin};

    use super::*;
    use crate::descriptors::{
        BoxedComponent, Cardinality, ComponentConstructionContext, ComponentDescriptor,
        ComponentFactoryDescriptor, ComponentScope, DependencyDescriptor, OperationKind,
        RpcCallContext, RpcDescriptor, RpcGroup, RpcOutcome, ServiceDescriptor, TypeDescriptor,
    };

    fn fake_factory<'a>(
        _: &'a mut ComponentConstructionContext,
    ) -> Pin<Box<dyn Future<Output = crate::Result<BoxedComponent>> + Send + 'a>> {
        Box::pin(async { todo!() })
    }

    /// Builds a one-dependency component descriptor at `$scope` whose single
    /// (explicit) factory carries `$dep`. A macro, not a fn, so each call site gets
    /// its own block-local `static` factory slice and a real fn pointer for
    /// `factories`. Argument order mirrors the former `scoped` fn:
    /// `scoped!(name, scope, dep, ty)`.
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
                hooks: $crate::hooks::no_hooks,
            }
        }};
    }

    fn fake_handler(
        _: RpcCallContext,
    ) -> Pin<
        Box<
            dyn Future<Output = core::result::Result<RpcOutcome, crate::extract::ErrorResponse>>
                + Send,
        >,
    > {
        Box::pin(async { todo!() })
    }

    // u8 = stand-in type for BackupRepository
    // u16 = stand-in type for PgPool
    // u32, u64 = stand-in types for RPC output types

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
        scope: ComponentScope::Singleton,
        factories: pg_pool_factories,
        hooks: crate::hooks::no_hooks,
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
        scope: ComponentScope::Singleton,
        factories: backup_repo_factories,
        hooks: crate::hooks::no_hooks,
    };

    static BACKUP_SERVICE_RPCS: [RpcDescriptor; 2] = [
        RpcDescriptor {
            name: "start_backup",
            operation: OperationKind::Unary,
            parameters: &[],
            output: TypeDescriptor::of::<u32>("JobId"),
            handler: fake_handler,
        },
        RpcDescriptor {
            name: "backup_status",
            operation: OperationKind::Unary,
            parameters: &[],
            output: TypeDescriptor::of::<u64>("BackupStatus"),
            handler: fake_handler,
        },
    ];

    static BACKUP_GROUPS: [RpcGroup; 1] = [RpcGroup {
        service: TypeDescriptor::of::<i32>("BackupService"),
        rpcs: &BACKUP_SERVICE_RPCS,
    }];

    fn backup_rpcs() -> &'static [RpcGroup] {
        &BACKUP_GROUPS
    }

    static BACKUP_SERVICE: ServiceDescriptor = ServiceDescriptor {
        id: "backup_service",
        name: "BackupService",
        ty: TypeDescriptor::of::<i32>("BackupService"),
        version: Some("1.0"),
        rpcs: backup_rpcs,
    };

    // i8 = stand-in type for a manually-registered service.
    static MANUAL_RPCS: [RpcDescriptor; 1] = [RpcDescriptor {
        name: "do_it",
        operation: OperationKind::Unary,
        parameters: &[],
        output: TypeDescriptor::of::<()>("()"),
        handler: fake_handler,
    }];

    static MANUAL_GROUPS: [RpcGroup; 1] = [RpcGroup {
        service: TypeDescriptor::of::<i8>("Manual"),
        rpcs: &MANUAL_RPCS,
    }];

    fn manual_rpcs() -> &'static [RpcGroup] {
        &MANUAL_GROUPS
    }

    fn no_rpcs() -> &'static [RpcGroup] {
        &[]
    }

    #[test]
    fn describe_groups_rpcs_under_service() {
        let registry = DescriptorRegistry {
            components: vec![BACKUP_REPO, PG_POOL],
            services: vec![BACKUP_SERVICE],
            ..Default::default()
        };
        let description = registry.to_string();

        assert!(description.contains("BackupRepository"));
        assert!(description.contains("BackupService"));
        assert!(description.contains("start_backup"));
        assert!(description.contains("backup_status"));
        assert!(
            description.contains("depends on: PgPool"),
            "dependency should appear in describe output"
        );

        let service_pos = description
            .find("BackupService")
            .expect("BackupService in output");
        let start_backup_pos = description
            .find("start_backup")
            .expect("start_backup in output");
        let backup_status_pos = description
            .find("backup_status")
            .expect("backup_status in output");

        assert!(
            service_pos < start_backup_pos,
            "start_backup should appear after BackupService header"
        );
        assert!(
            service_pos < backup_status_pos,
            "backup_status should appear after BackupService header"
        );
    }

    #[test]
    fn validate_passes_with_fulfilled_dependencies() {
        let registry = DescriptorRegistry {
            components: vec![BACKUP_REPO, PG_POOL],
            services: vec![BACKUP_SERVICE],
            ..Default::default()
        };

        assert!(registry.validate().is_ok());
    }

    #[test]
    fn validate_detects_duplicate_component_ids() {
        let registry = DescriptorRegistry {
            components: vec![BACKUP_REPO, BACKUP_REPO],
            ..Default::default()
        };

        assert!(registry.validate().is_err());
    }

    #[test]
    fn validate_detects_missing_dependency() {
        let registry = DescriptorRegistry {
            components: vec![BACKUP_REPO],
            ..Default::default()
        };

        assert!(registry.validate().is_err());
    }

    #[test]
    fn validate_accepts_manual_component_descriptor() {
        // BackupRepository depends on PgPool (u16). Absent, validation fails; a
        // factory-less descriptor (as `with_component` synthesizes) satisfies it.
        let without = DescriptorRegistry {
            components: vec![BACKUP_REPO],
            ..Default::default()
        };

        assert!(without.validate().is_err());

        let manual = ComponentDescriptor::manual(
            "pg_pool_manual",
            "PgPool",
            TypeDescriptor::of::<u16>("PgPool"),
            ComponentScope::Singleton,
        );
        let with = DescriptorRegistry {
            components: vec![BACKUP_REPO, manual],
            ..Default::default()
        };

        assert!(with.validate().is_ok());
    }

    #[test]
    fn manual_service_registration_validates() {
        // The shape `with_service(instance).rpcs(..)` produces: a header, a
        // factory-less component (the provided instance), and an RPC group — all
        // for the same type.
        let service = ServiceDescriptor {
            id: "manual",
            name: "Manual",
            ty: TypeDescriptor::of::<i8>("Manual"),
            version: Some("1.0"),
            rpcs: manual_rpcs,
        };
        let component = ComponentDescriptor::manual(
            "manual",
            "Manual",
            TypeDescriptor::of::<i8>("Manual"),
            ComponentScope::Singleton,
        );

        let registry = DescriptorRegistry {
            components: vec![component],
            services: vec![service],
            ..Default::default()
        };

        assert!(registry.validate().is_ok());
        assert_eq!(registry.resolved_services()[0].rpcs.len(), 1);
    }

    #[test]
    fn validate_detects_empty_service() {
        static EMPTY_SERVICE: ServiceDescriptor = ServiceDescriptor {
            id: "empty",
            name: "EmptyService",
            ty: TypeDescriptor::of::<i64>("EmptyService"),
            version: None,
            rpcs: no_rpcs,
        };

        let registry = DescriptorRegistry {
            services: vec![EMPTY_SERVICE],
            ..Default::default()
        };

        assert!(registry.validate().is_err());
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
    }];

    static REQUEST_DEP_ON_CONNECTION: [DependencyDescriptor; 1] = [DependencyDescriptor {
        name: "ConnComp",
        ty: TypeDescriptor::of::<i32>("ConnComp"),
        cardinality: Cardinality::One,
        optional: false,
        dynamic: false,
        qualifier: None,
        config: false,
    }];

    #[test]
    fn validate_rejects_singleton_depending_on_request() {
        // A singleton outlives a request-scoped instance, so the captive-dependency
        // rule forbids holding one.
        let request = scoped!(
            "ReqComp",
            ComponentScope::Request,
            &[],
            TypeDescriptor::of::<i64>("ReqComp"),
        );
        let singleton = scoped!(
            "RootComp",
            ComponentScope::Singleton,
            &SINGLETON_DEP_ON_REQUEST,
            TypeDescriptor::of::<i16>("RootComp"),
        );

        let registry = DescriptorRegistry {
            components: vec![singleton, request],
            ..Default::default()
        };

        // Go through the full `validate()` to confirm scope checking is wired into
        // the path `Daemon::build` runs.
        assert!(matches!(
            registry.validate(),
            Err(Error::ScopeViolation { .. })
        ));
    }

    #[test]
    fn validate_allows_request_depending_on_connection() {
        // A request-scoped component may depend on a longer-lived connection one.
        let connection = scoped!(
            "ConnComp",
            ComponentScope::Connection,
            &[],
            TypeDescriptor::of::<i32>("ConnComp"),
        );
        let request = scoped!(
            "ReqComp",
            ComponentScope::Request,
            &REQUEST_DEP_ON_CONNECTION,
            TypeDescriptor::of::<i64>("ReqComp"),
        );

        let registry = DescriptorRegistry {
            components: vec![connection, request],
            ..Default::default()
        };

        assert!(registry.validate_scopes(&registry.components).is_ok());
    }

    // --- Config validation -------------------------------------------------
    //
    // u128 = stand-in type for a config struct (e.g. DbConfig).

    fn dummy_bind(
        _: &crate::config::ConfigManager,
        _: &str,
    ) -> Result<BoxedComponent, crate::config::ConfigError> {
        unreachable!("validation never binds")
    }

    fn dummy_slot(
        _: &BoxedComponent,
        _: &str,
    ) -> Option<Box<dyn crate::config::ReloadableConfig>> {
        unreachable!("validation never builds reload slots")
    }

    fn config_binding(path: &str) -> ConfigBinding {
        ConfigBinding {
            ty: TypeDescriptor::of::<u128>("DbConfig"),
            path: path.to_string(),
            bind: dummy_bind,
            slot: dummy_slot,
            defaults: crate::config::DefaultSpec::None,
        }
    }

    static CONFIG_DEP_PATHED: [DependencyDescriptor; 1] = [DependencyDescriptor {
        name: "DbConfig",
        ty: TypeDescriptor::of::<u128>("DbConfig"),
        cardinality: Cardinality::One,
        optional: false,
        dynamic: false,
        qualifier: Some("app.db.reader"),
        config: true,
    }];

    static CONFIG_DEP_SHORTHAND: [DependencyDescriptor; 1] = [DependencyDescriptor {
        name: "DbConfig",
        ty: TypeDescriptor::of::<u128>("DbConfig"),
        cardinality: Cardinality::One,
        optional: false,
        dynamic: false,
        qualifier: None,
        config: true,
    }];

    #[test]
    fn validate_rejects_unbound_config_path() {
        let consumer = scoped!(
            "Pools",
            ComponentScope::Singleton,
            &CONFIG_DEP_PATHED,
            TypeDescriptor::of::<i16>("Pools"),
        );

        let registry = DescriptorRegistry {
            components: vec![consumer],
            ..Default::default()
        };

        assert!(matches!(
            registry.validate(),
            Err(Error::MissingConfig { .. })
        ));
    }

    #[test]
    fn validate_accepts_bound_config_path() {
        let consumer = scoped!(
            "Pools",
            ComponentScope::Singleton,
            &CONFIG_DEP_PATHED,
            TypeDescriptor::of::<i16>("Pools"),
        );

        let registry = DescriptorRegistry {
            components: vec![consumer],
            config_bindings: vec![config_binding("app.db.reader")],
            ..Default::default()
        };

        assert!(registry.validate().is_ok());
    }

    #[test]
    fn validate_rejects_ambiguous_config_shorthand() {
        // Two bindings of the same type with an unpathed `#[config]` edge is
        // ambiguous — the shorthand only works for a single binding.
        let consumer = scoped!(
            "Pools",
            ComponentScope::Singleton,
            &CONFIG_DEP_SHORTHAND,
            TypeDescriptor::of::<i16>("Pools"),
        );

        let registry = DescriptorRegistry {
            components: vec![consumer],
            config_bindings: vec![
                config_binding("app.db.reader"),
                config_binding("app.db.writer"),
            ],
            ..Default::default()
        };

        assert!(matches!(
            registry.validate(),
            Err(Error::AmbiguousConfig { count: 2, .. })
        ));
    }

    #[test]
    fn validate_accepts_sole_config_shorthand() {
        let consumer = scoped!(
            "Pools",
            ComponentScope::Singleton,
            &CONFIG_DEP_SHORTHAND,
            TypeDescriptor::of::<i16>("Pools"),
        );

        let registry = DescriptorRegistry {
            components: vec![consumer],
            config_bindings: vec![config_binding("app.db")],
            ..Default::default()
        };

        assert!(registry.validate().is_ok());
    }

    #[test]
    fn validate_rejects_transient_depending_on_connection() {
        // A transient may depend only on singletons in v1, so a connection-scoped
        // dependency is rejected.
        let connection = scoped!(
            "ConnComp",
            ComponentScope::Connection,
            &[],
            TypeDescriptor::of::<i32>("ConnComp"),
        );
        let transient = scoped!(
            "TransComp",
            ComponentScope::Transient,
            &REQUEST_DEP_ON_CONNECTION,
            TypeDescriptor::of::<i64>("TransComp"),
        );

        let registry = DescriptorRegistry {
            components: vec![connection, transient],
            ..Default::default()
        };

        assert!(matches!(
            registry.validate_scopes(&registry.components),
            Err(Error::ScopeViolation { .. })
        ));
    }
}
