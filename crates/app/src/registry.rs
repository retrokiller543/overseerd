use std::any::TypeId;
use std::collections::HashMap;
use std::fmt;
use std::fmt::Write;

use overseerd_config::{CONFIG_BINDINGS, ConfigBinding};
use overseerd_core::DependencyDescriptor;
use overseerd_di::{
    COMPONENTS, ComponentDescriptor, ComponentRegistry, PROVIDERS, ProviderDescriptor,
};

use crate::error::Error;

/// Holds the *agnostic* component, provider, and config-binding descriptors of an app —
/// declarations only. Runtime instances live in the
/// [`ScopeContainer`](overseerd_di::ScopeContainer).
///
/// Wraps the DI engine's [`ComponentRegistry`] (component/provider graph) with the config
/// bindings, and runs the cross-cutting validation the component graph alone cannot
/// (config edges). Protocol-specific declarations (services/routes) live in the protocol
/// plugin, not here, so this stays usable by any protocol.
#[derive(Default, Debug)]
pub struct AppRegistry {
    pub components: Vec<ComponentDescriptor>,
    pub providers: Vec<ProviderDescriptor>,
    /// Config bindings (a config type bound to a property path). Populated from the
    /// auto-discovered config bindings slice and from explicit builder bindings.
    pub config_bindings: Vec<ConfigBinding>,
}

impl AppRegistry {
    /// Collects every link-time-registered agnostic descriptor (components, providers,
    /// config bindings) into an `AppRegistry`. Protocol variant slices (e.g. RPC services)
    /// are folded in by the protocol plugin, not here.
    pub fn collect() -> Self {
        Self {
            components: COMPONENTS.iter().copied().collect(),
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

    /// Collapses the registered descriptors to one per type (delegated to the DI engine).
    pub fn resolved_components(&self) -> crate::Result<Vec<ComponentDescriptor>> {
        Ok(self.component_registry().resolved_components()?)
    }

    /// Validates structural consistency: the component graph (via the DI engine), then the
    /// config-binding rules.
    pub fn validate(&self) -> crate::Result<()> {
        self.component_registry().validate()?;

        let components = self.resolved_components()?;

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
}

impl fmt::Display for AppRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_components(f)
    }
}
