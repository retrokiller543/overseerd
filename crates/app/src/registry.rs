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
        let mut components: Vec<_> = COMPONENTS.iter().copied().collect();
        let mut providers: Vec<_> = PROVIDERS.iter().copied().collect();
        let mut config_bindings: Vec<_> = CONFIG_BINDINGS.iter().map(|d| d.to_binding()).collect();

        // Distributed-slice order differs between linkers. Sort auto-discovered
        // descriptors only; explicit builder registrations retain caller order.
        components.sort_by_key(|component| component.id);
        providers.sort_by(|left, right| {
            (left.trait_ty.type_name)()
                .cmp((right.trait_ty.type_name)())
                .then_with(|| (left.concrete_ty.type_name)().cmp((right.concrete_ty.type_name)()))
                .then_with(|| left.qualifier.cmp(right.qualifier))
        });
        config_bindings.sort_by(|left, right| {
            (left.ty.type_name)()
                .cmp((right.ty.type_name)())
                .then_with(|| left.path.cmp(&right.path))
        });

        Self {
            components,
            providers,
            config_bindings,
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

                        match bound_paths.as_slice() {
                            [_] => {}

                            [] => {
                                return Err(Error::MissingConfig {
                                    component: c.name.to_string(),
                                    type_name: (dep.ty.type_name)().to_string(),
                                    path: "<unqualified>".to_string(),
                                });
                            }

                            _ => {
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

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use overseerd_config::{ConfigBinding, ConfigProperties};
    use overseerd_core::{Cardinality, DependencyDescriptor, TypeDescriptor};
    use overseerd_di::{
        BoxedComponent, ComponentConstructionContext, ComponentDescriptor,
        ComponentFactoryDescriptor, Singleton,
    };

    use super::AppRegistry;
    use crate::Error;

    #[derive(serde::Deserialize)]
    struct TestConfig;

    impl ConfigProperties for TestConfig {
        const NAME: &'static str = "TestConfig";
    }

    fn fake_factory<'a>(
        _: &'a mut ComponentConstructionContext,
    ) -> Pin<Box<dyn Future<Output = overseerd_di::Result<BoxedComponent>> + Send + 'a>> {
        Box::pin(async { unreachable!("registry validation does not construct components") })
    }

    fn config_deps() -> Vec<DependencyDescriptor> {
        vec![DependencyDescriptor {
            name: "cfg",
            ty: TypeDescriptor::of::<TestConfig>("TestConfig"),
            cardinality: Cardinality::One,
            optional: false,
            dynamic: false,
            qualifier: None,
            config: true,
        }]
    }

    static CONFIG_FACTORY: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
        construct: fake_factory,
        dependencies: config_deps,
        default: false,
    }];

    fn config_factories() -> &'static [ComponentFactoryDescriptor] {
        &CONFIG_FACTORY
    }

    fn component() -> ComponentDescriptor {
        ComponentDescriptor {
            id: "needs_config",
            name: "NeedsConfig",
            ty: TypeDescriptor::of::<()>("NeedsConfig"),
            scope: &Singleton,
            factories: config_factories,
            hooks: overseerd_hooks::no_hooks,
        }
    }

    #[test]
    fn missing_unqualified_config_binding_is_missing_not_ambiguous() {
        let registry = AppRegistry {
            components: vec![component()],
            providers: Vec::new(),
            config_bindings: Vec::new(),
        };

        let err = registry.validate().expect_err("config binding is missing");

        assert!(matches!(
            err,
            Error::MissingConfig {
                component,
                type_name,
                path,
            } if component == "NeedsConfig"
                && type_name.ends_with("TestConfig")
                && path == "<unqualified>"
        ));
    }

    #[test]
    fn multiple_unqualified_config_bindings_are_ambiguous() {
        let registry = AppRegistry {
            components: vec![component()],
            providers: Vec::new(),
            config_bindings: vec![
                ConfigBinding::of::<TestConfig>("one"),
                ConfigBinding::of::<TestConfig>("two"),
            ],
        };

        let err = registry
            .validate()
            .expect_err("config binding is ambiguous");

        assert!(matches!(
            err,
            Error::AmbiguousConfig {
                component,
                type_name,
                count: 2,
                paths,
            } if component == "NeedsConfig"
                && type_name.ends_with("TestConfig")
                && paths == "one, two"
        ));
    }
}
