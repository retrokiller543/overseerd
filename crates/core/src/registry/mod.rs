use crate::{
    DependencyDescriptor, ParameterDescriptor, RpcDescriptor,
    descriptors::{COMPONENTS, ComponentDescriptor, RPC_GROUPS, RpcGroup, SERVICES, ServiceDescriptor},
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
    pub rpc_groups: Vec<RpcGroup>,
}

/// A service header with its RPCs assembled from every matching `RpcGroup`.
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
            rpc_groups: RPC_GROUPS.iter().copied().collect(),
        }
    }

    /// Assembles each service header with the RPCs contributed to its type.
    pub fn resolved_services(&self) -> Vec<ResolvedService> {
        self.services
            .iter()
            .map(|descriptor| {
                let service_ty = (descriptor.ty.type_id)();
                let rpcs = self
                    .rpc_groups
                    .iter()
                    .filter(|group| (group.service.type_id)() == service_ty)
                    .flat_map(|group| group.rpcs.iter())
                    .collect();

                ResolvedService {
                    descriptor: *descriptor,
                    rpcs,
                }
            })
            .collect()
    }

    /// Selects the effective component per type: an explicit factory (`#[init]`
    /// or a hand-written descriptor) overrides a default field-injection one.
    /// Two explicit factories for the same type is an error.
    pub fn resolved_components(&self) -> crate::Result<Vec<ComponentDescriptor>> {
        let mut chosen: HashMap<TypeId, ComponentDescriptor> = HashMap::new();

        for component in &self.components {
            let type_id = (component.ty.type_id)();

            match chosen.get(&type_id) {
                None => {
                    chosen.insert(type_id, *component);
                }

                Some(existing) => {
                    if existing.default_factory && !component.default_factory {
                        chosen.insert(type_id, *component);
                    } else if !existing.default_factory && !component.default_factory {
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
        self.validate_rpc_groups()?;
        self.validate_services()?;
        self.validate_dependencies(&components)?;

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

    fn validate_rpc_groups(&self) -> crate::Result<()> {
        let service_types: HashSet<TypeId> =
            self.services.iter().map(|s| (s.ty.type_id)()).collect();

        for group in &self.rpc_groups {
            if !service_types.contains(&(group.service.type_id)()) {
                return Err(Error::OrphanRpcs((group.service.type_name)().to_string()));
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

        for c in components {
            for dep in c.dependencies {
                // Multi-valued edges (Collection/Keyed) accept zero providers;
                // `optional` tolerates absence; `dynamic` providers are supplied
                // at runtime and so are exempt from static validation.
                let must_exist = dep.cardinality.requires_provider() && !dep.optional && !dep.dynamic;

                if must_exist && !available.contains(&(dep.ty.type_id)()) {
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

            if !c.dependencies.is_empty() {
                Self::write_dependency(f, c.dependencies.iter())?
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
        ComponentScope, DependencyDescriptor, OperationKind, RpcCallContext, RpcDescriptor,
        RpcGroup, RpcOutcome, ServiceDescriptor, TypeDescriptor,
    };

    fn fake_factory<'a>(
        _: &'a mut ComponentConstructionContext,
    ) -> Pin<Box<dyn Future<Output = crate::Result<BoxedComponent>> + Send + 'a>> {
        Box::pin(async { todo!() })
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

    static PG_POOL_DEPS: [DependencyDescriptor; 0] = [];

    static PG_POOL: ComponentDescriptor = ComponentDescriptor {
        id: "pg_pool",
        name: "PgPool",
        ty: TypeDescriptor::of::<u16>("PgPool"),
        scope: ComponentScope::Singleton,
        dependencies: &PG_POOL_DEPS,
        factory: Some(fake_factory),
        default_factory: false,
    };

    static BACKUP_REPO_DEPS: [DependencyDescriptor; 1] = [DependencyDescriptor {
        name: "PgPool",
        ty: TypeDescriptor::of::<u16>("PgPool"),
        cardinality: Cardinality::One,
        optional: false,
        dynamic: false,
    }];

    static BACKUP_REPO: ComponentDescriptor = ComponentDescriptor {
        id: "backup_repo",
        name: "BackupRepository",
        ty: TypeDescriptor::of::<u8>("BackupRepository"),
        scope: ComponentScope::Singleton,
        dependencies: &BACKUP_REPO_DEPS,
        factory: Some(fake_factory),
        default_factory: false,
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

    static BACKUP_SERVICE: ServiceDescriptor = ServiceDescriptor {
        id: "backup_service",
        name: "BackupService",
        ty: TypeDescriptor::of::<i32>("BackupService"),
        version: Some("1.0"),
    };

    static BACKUP_RPCS_GROUP: RpcGroup = RpcGroup {
        service: TypeDescriptor::of::<i32>("BackupService"),
        rpcs: &BACKUP_SERVICE_RPCS,
    };

    // i8 = stand-in type for a manually-registered service.
    static MANUAL_RPCS: [RpcDescriptor; 1] = [RpcDescriptor {
        name: "do_it",
        operation: OperationKind::Unary,
        parameters: &[],
        output: TypeDescriptor::of::<()>("()"),
        handler: fake_handler,
    }];

    #[test]
    fn describe_groups_rpcs_under_service() {
        let registry = DescriptorRegistry {
            components: vec![BACKUP_REPO, PG_POOL],
            services: vec![BACKUP_SERVICE],
            rpc_groups: vec![BACKUP_RPCS_GROUP],
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
            rpc_groups: vec![BACKUP_RPCS_GROUP],
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

        let manual = ComponentDescriptor {
            id: "pg_pool_manual",
            name: "PgPool",
            ty: TypeDescriptor::of::<u16>("PgPool"),
            scope: ComponentScope::Singleton,
            dependencies: &[],
            factory: None,
            default_factory: false,
        };
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
        };
        let component = ComponentDescriptor {
            id: "manual",
            name: "Manual",
            ty: TypeDescriptor::of::<i8>("Manual"),
            scope: ComponentScope::Singleton,
            dependencies: &[],
            factory: None,
            default_factory: false,
        };
        let group = RpcGroup {
            service: TypeDescriptor::of::<i8>("Manual"),
            rpcs: &MANUAL_RPCS,
        };

        let registry = DescriptorRegistry {
            components: vec![component],
            services: vec![service],
            rpc_groups: vec![group],
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
        };

        let registry = DescriptorRegistry {
            services: vec![EMPTY_SERVICE],
            ..Default::default()
        };

        assert!(registry.validate().is_err());
    }
}
