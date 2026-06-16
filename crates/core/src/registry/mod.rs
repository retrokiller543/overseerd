use std::collections::HashSet;

use crate::{
    descriptors::{ComponentDescriptor, Descriptor, ServiceDescriptor},
    error::Error,
};

/// Runtime registry built by collecting static inventory descriptors.
///
/// Owns `Vec` allocations for the collected descriptor references; the
/// underlying descriptors remain in static storage.
#[derive(Debug)]
pub struct Registry {
    pub components: Vec<&'static ComponentDescriptor>,
    pub services: Vec<&'static ServiceDescriptor>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            components: Vec::new(),
            services: Vec::new(),
        }
    }
}

impl Registry {
    /// Collects all submitted inventory descriptors into a Registry.
    pub fn collect() -> Self {
        let mut registry = Self::default();
        for descriptor in inventory::iter::<Descriptor> {
            match descriptor {
                Descriptor::Component(c) => registry.components.push(c),
                Descriptor::Service(s) => registry.services.push(s),
            }
        }
        registry
    }

    /// Validates structural consistency of the registry.
    pub fn validate(&self) -> crate::Result<()> {
        self.validate_component_ids()?;
        self.validate_services()?;
        self.validate_dependencies()?;
        Ok(())
    }

    /// Returns a human-readable description of all registered components, services, and RPCs.
    pub fn describe(&self) -> String {
        let mut out = String::new();

        out.push_str("Components:\n");
        for c in &self.components {
            out.push_str(&format!("  {}\n", c.name));
            if !c.dependencies.is_empty() {
                let deps: Vec<&str> = c.dependencies.iter().map(|d| d.name).collect();
                out.push_str(&format!("    depends on: {}\n", deps.join(", ")));
            }
        }

        out.push_str("\nServices:\n");
        for s in &self.services {
            if let Some(v) = s.version {
                out.push_str(&format!("  {} (v{})\n", s.name, v));
            } else {
                out.push_str(&format!("  {}\n", s.name));
            }
            for rpc in s.rpcs {
                let params: Vec<&str> = rpc.parameters.iter().map(|p| p.ty.name).collect();
                out.push_str(&format!(
                    "    rpc {}({}) -> {}\n",
                    rpc.name,
                    params.join(", "),
                    rpc.output.name,
                ));
            }
        }

        out
    }

    fn validate_component_ids(&self) -> crate::Result<()> {
        let mut seen = HashSet::new();
        for c in &self.components {
            if !seen.insert(c.id) {
                return Err(Error::DuplicateComponentId(c.id.to_string()));
            }
        }
        Ok(())
    }

    fn validate_services(&self) -> crate::Result<()> {
        let mut seen_ids = HashSet::new();
        let mut seen_paths: HashSet<String> = HashSet::new();

        for s in &self.services {
            if !seen_ids.insert(s.id) {
                return Err(Error::DuplicateServiceId(s.id.to_string()));
            }

            if s.rpcs.is_empty() {
                return Err(Error::EmptyService(s.name.to_string()));
            }

            let mut seen_rpc_names = HashSet::new();
            for rpc in s.rpcs {
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

    fn validate_dependencies(&self) -> crate::Result<()> {
        let available: HashSet<std::any::TypeId> =
            self.components.iter().map(|c| (c.ty.type_id)()).collect();

        for c in &self.components {
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
}

#[cfg(test)]
mod tests {
    use std::{future::Future, pin::Pin};

    use super::*;
    use crate::descriptors::{
        BoxedComponent, ComponentConstructionContext, ComponentDescriptor, ComponentScope,
        DependencyDescriptor, OperationKind, RpcCallContext, RpcDescriptor, RpcResponse,
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
    };

    static BACKUP_SERVICE_RPCS: [RpcDescriptor; 2] = [
        RpcDescriptor {
            name: "start_backup",
            operation: OperationKind::Command,
            parameters: &[],
            output: TypeDescriptor::of::<u32>("JobId"),
            handler: fake_handler,
        },
        RpcDescriptor {
            name: "backup_status",
            operation: OperationKind::Query,
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
        rpcs: &BACKUP_SERVICE_RPCS,
    };

    #[test]
    fn describe_groups_rpcs_under_service() {
        let registry = Registry {
            components: vec![&BACKUP_REPO, &PG_POOL],
            services: vec![&BACKUP_SERVICE],
        };

        let description = registry.describe();

        assert!(description.contains("BackupRepository"));
        assert!(description.contains("BackupService"));

        let service_pos = description.find("BackupService").unwrap();
        let start_backup_pos = description.find("start_backup").unwrap();
        let backup_status_pos = description.find("backup_status").unwrap();

        assert!(
            service_pos < start_backup_pos,
            "start_backup should appear after BackupService header"
        );
        assert!(
            service_pos < backup_status_pos,
            "backup_status should appear after BackupService header"
        );
        assert!(
            description.contains("depends on: PgPool"),
            "dependency should appear in describe output"
        );
    }

    #[test]
    fn validate_passes_with_fulfilled_dependencies() {
        let registry = Registry {
            components: vec![&BACKUP_REPO, &PG_POOL],
            services: vec![&BACKUP_SERVICE],
        };
        assert!(registry.validate().is_ok());
    }

    #[test]
    fn validate_detects_duplicate_component_ids() {
        let registry = Registry {
            components: vec![&BACKUP_REPO, &BACKUP_REPO],
            services: vec![],
        };
        assert!(registry.validate().is_err());
    }

    #[test]
    fn validate_detects_missing_dependency() {
        let registry = Registry {
            components: vec![&BACKUP_REPO],
            services: vec![],
        };
        assert!(registry.validate().is_err());
    }

    #[test]
    fn validate_detects_empty_service() {
        static EMPTY_SERVICE: ServiceDescriptor = ServiceDescriptor {
            id: "empty",
            name: "EmptyService",
            ty: TypeDescriptor::of::<i64>("EmptyService"),
            version: None,
            rpcs: &[],
        };

        let registry = Registry {
            components: vec![],
            services: vec![&EMPTY_SERVICE],
        };
        assert!(registry.validate().is_err());
    }
}