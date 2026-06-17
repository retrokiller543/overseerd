use crate::{
    DependencyDescriptor, ParameterDescriptor, RpcDescriptor,
    descriptors::{ComponentDescriptor, Descriptor, RpcGroup, ServiceDescriptor},
    error::Error,
};
use std::any::TypeId;
use std::collections::HashMap;
use std::fmt::Write;
use std::{collections::HashSet, fmt};

/// Runtime registry built by collecting static inventory descriptors.
///
/// Owns `Vec` allocations for the collected descriptor references; the
/// underlying descriptors remain in static storage. Services are headers tied
/// to a type; their RPCs are contributed by `rpc_groups` and assembled on
/// demand via [`resolved_services`](Self::resolved_services).
#[derive(Default, Debug)]
pub struct Registry {
    pub components: Vec<&'static ComponentDescriptor>,
    pub services: Vec<&'static ServiceDescriptor>,
    pub rpc_groups: Vec<&'static RpcGroup>,
}

/// A service header with its RPCs assembled from every matching `RpcGroup`.
pub struct ResolvedService {
    pub descriptor: &'static ServiceDescriptor,
    pub rpcs: Vec<&'static RpcDescriptor>,
}

impl Registry {
    /// Collects all submitted inventory descriptors into a Registry.
    pub fn collect() -> Self {
        let mut registry = Self::default();
        for descriptor in inventory::iter::<Descriptor> {
            match descriptor {
                Descriptor::Component(c) => registry.components.push(c),
                Descriptor::Service(s) => registry.services.push(s),
                Descriptor::Rpcs(g) => registry.rpc_groups.push(g),
            }
        }

        registry
    }

    /// Assembles each service header with the RPCs contributed to its type.
    pub fn resolved_services(&self) -> Vec<ResolvedService> {
        self.services
            .iter()
            .map(|&descriptor| {
                let service_ty = (descriptor.ty.type_id)();
                let rpcs = self
                    .rpc_groups
                    .iter()
                    .filter(|group| (group.service.type_id)() == service_ty)
                    .flat_map(|group| group.rpcs.iter())
                    .collect();

                ResolvedService { descriptor, rpcs }
            })
            .collect()
    }

    /// Selects the effective component per type: an explicit factory (`#[init]`
    /// or a hand-written descriptor) overrides a default field-injection one.
    /// Two explicit factories for the same type is an error.
    pub fn resolved_components(&self) -> crate::Result<Vec<&'static ComponentDescriptor>> {
        let mut chosen: HashMap<TypeId, &'static ComponentDescriptor> = HashMap::new();

        for &component in &self.components {
            let type_id = (component.ty.type_id)();

            match chosen.get(&type_id) {
                None => {
                    chosen.insert(type_id, component);
                }

                Some(existing) => {
                    if existing.default_factory && !component.default_factory {
                        chosen.insert(type_id, component);
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
    pub fn validate(&self) -> crate::Result<()> {
        self.validate_with(&HashSet::new())
    }

    /// Like [`validate`](Self::validate), but treats `external` component types
    /// (e.g. instances supplied via `DaemonBuilder::with_component`) as
    /// available when checking dependencies, since they are seeded into the
    /// container before factory-built components are constructed.
    pub fn validate_with(&self, external: &HashSet<TypeId>) -> crate::Result<()> {
        let components = self.resolved_components()?;

        self.validate_component_ids(&components)?;
        self.validate_rpc_groups()?;
        self.validate_services()?;
        self.validate_dependencies(&components, external)?;

        Ok(())
    }

    fn validate_component_ids(&self, components: &[&'static ComponentDescriptor]) -> crate::Result<()> {
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

    fn validate_dependencies(
        &self,
        components: &[&'static ComponentDescriptor],
        external: &HashSet<TypeId>,
    ) -> crate::Result<()> {
        let mut available: HashSet<TypeId> =
            components.iter().map(|c| (c.ty.type_id)()).collect();

        available.extend(external.iter().copied());

        for c in components {
            for dep in c.dependencies {
                if !dep.optional && !available.contains(&(dep.ty.type_id)()) {
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

impl fmt::Display for Registry {
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
        BoxedComponent, ComponentConstructionContext, ComponentDescriptor, ComponentScope,
        DependencyDescriptor, OperationKind, RpcCallContext, RpcDescriptor, RpcGroup, RpcResponse,
        ServiceDescriptor, TypeDescriptor,
    };

    fn fake_factory<'a>(
        _: &'a mut ComponentConstructionContext,
    ) -> Pin<Box<dyn Future<Output = crate::Result<BoxedComponent>> + Send + 'a>> {
        Box::pin(async { todo!() })
    }

    fn fake_handler(
        _: RpcCallContext,
    ) -> Pin<Box<dyn Future<Output = crate::Result<RpcResponse>> + Send>> {
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
        factory: fake_factory,
        default_factory: false,
    };

    static BACKUP_REPO_DEPS: [DependencyDescriptor; 1] = [DependencyDescriptor {
        name: "PgPool",
        ty: TypeDescriptor::of::<u16>("PgPool"),
        optional: false,
    }];

    static BACKUP_REPO: ComponentDescriptor = ComponentDescriptor {
        id: "backup_repo",
        name: "BackupRepository",
        ty: TypeDescriptor::of::<u8>("BackupRepository"),
        scope: ComponentScope::Singleton,
        dependencies: &BACKUP_REPO_DEPS,
        factory: fake_factory,
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

    #[test]
    fn describe_groups_rpcs_under_service() {
        let registry = Registry {
            components: vec![&BACKUP_REPO, &PG_POOL],
            services: vec![&BACKUP_SERVICE],
            rpc_groups: vec![&BACKUP_RPCS_GROUP],
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
        let registry = Registry {
            components: vec![&BACKUP_REPO, &PG_POOL],
            services: vec![&BACKUP_SERVICE],
            rpc_groups: vec![&BACKUP_RPCS_GROUP],
        };

        assert!(registry.validate().is_ok());
    }

    #[test]
    fn validate_detects_duplicate_component_ids() {
        let registry = Registry {
            components: vec![&BACKUP_REPO, &BACKUP_REPO],
            services: vec![],
            rpc_groups: vec![],
        };

        assert!(registry.validate().is_err());
    }

    #[test]
    fn validate_detects_missing_dependency() {
        let registry = Registry {
            components: vec![&BACKUP_REPO],
            services: vec![],
            rpc_groups: vec![],
        };

        assert!(registry.validate().is_err());
    }

    #[test]
    fn validate_with_accepts_externally_provided_dependency() {
        let registry = Registry {
            components: vec![&BACKUP_REPO],
            services: vec![],
            rpc_groups: vec![],
        };

        // BackupRepository depends on PgPool (u16). Absent from the registry,
        // it fails plain validation, but a `with_component`-supplied instance
        // (modeled as an external type id) satisfies it.
        let external = HashSet::from([std::any::TypeId::of::<u16>()]);

        assert!(registry.validate().is_err());
        assert!(registry.validate_with(&external).is_ok());
    }

    #[test]
    fn validate_detects_empty_service() {
        static EMPTY_SERVICE: ServiceDescriptor = ServiceDescriptor {
            id: "empty",
            name: "EmptyService",
            ty: TypeDescriptor::of::<i64>("EmptyService"),
            version: None,
        };

        let registry = Registry {
            components: vec![],
            services: vec![&EMPTY_SERVICE],
            rpc_groups: vec![],
        };

        assert!(registry.validate().is_err());
    }
}
