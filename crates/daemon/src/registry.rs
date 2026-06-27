use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fmt::Write;

use overseerd_config::{CONFIG_BINDINGS, ConfigBinding};
use overseerd_core::DependencyDescriptor;
use overseerd_di::{
    COMPONENTS, ComponentDescriptor, ComponentRegistry, PROVIDERS, ProviderDescriptor,
};

use crate::descriptors::{ParameterDescriptor, RpcDescriptor, SERVICES, ServiceDescriptor};
use crate::error::Error;

/// Holds the component, service, and RPC *descriptors* for a daemon — declarations only.
/// Runtime instances live in the [`ScopeContainer`](overseerd_di::ScopeContainer).
///
/// Wraps the DI engine's [`ComponentRegistry`] (component/provider graph) with the
/// daemon's own service, RPC, and config-binding metadata, and runs the cross-cutting
/// validation (services, RPC paths, config edges) the component graph alone cannot.
#[derive(Default, Debug)]
pub struct DescriptorRegistry {
    pub components: Vec<ComponentDescriptor>,
    pub services: Vec<ServiceDescriptor>,
    pub providers: Vec<ProviderDescriptor>,
    /// Config bindings (a config type bound to a property path). Populated from the
    /// auto-discovered config bindings slice and from explicit builder bindings.
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

    /// The DI engine's view of this registry — the component/provider graph.
    fn component_registry(&self) -> ComponentRegistry {
        ComponentRegistry {
            components: self.components.clone(),
            providers: self.providers.clone(),
        }
    }

    /// Assembles each service header with the RPCs it owns. Services are deduped by type.
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

    /// Collapses the registered descriptors to one per type (delegated to the DI engine).
    pub fn resolved_components(&self) -> crate::Result<Vec<ComponentDescriptor>> {
        Ok(self.component_registry().resolved_components()?)
    }

    /// Validates structural consistency: the component graph (via the DI engine), then the
    /// service/RPC and config-binding rules the daemon owns.
    pub fn validate(&self) -> crate::Result<()> {
        self.component_registry().validate()?;

        let components = self.resolved_components()?;

        self.validate_services()?;
        self.validate_configs(&components)?;

        Ok(())
    }

    /// Validates config edges against the registered bindings: a `#[config("path")]` edge
    /// must have a binding of its type at that path, and a `#[config]` shorthand edge must
    /// have exactly one binding of its type.
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

impl fmt::Display for DescriptorRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_components(f)?;
        writeln!(f)?;
        self.write_services(f)?;

        Ok(())
    }
}
