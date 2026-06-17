use std::{
    any::{Any, TypeId},
    collections::{HashMap, HashSet},
    sync::Arc,
};

use tracing::{debug, error, info, instrument, trace};

use crate::{
    BoxedComponent, Error,
    descriptors::{ComponentDescriptor, component::ComponentConstructionContext},
};

/// Holds all fully constructed component instances for a running daemon.
pub struct ComponentContainer {
    components: HashMap<TypeId, crate::BoxedComponent>,
}

impl ComponentContainer {
    /// Resolves all registered components in dependency order and returns a built ComponentContainer.
    ///
    /// `components` is the effective component set (after default/override
    /// resolution). `manual` holds pre-built instances supplied at the builder
    /// (e.g. a service constructed by hand); they are seeded first, so
    /// factory-built components may depend on them.
    #[instrument(skip_all, fields(count = components.len()))]
    pub async fn build(
        components: &[ComponentDescriptor],
        instances: Vec<BoxedComponent>,
    ) -> crate::Result<Self> {
        debug!("resolving component dependency order");

        let mut ctx = ComponentConstructionContext::new();
        let mut prebuilt: HashSet<TypeId> = HashSet::new();

        for component in instances {
            prebuilt.insert((component.ty.type_id)());
            ctx.insert(component);
        }

        let sorted = topological_sort(components, &prebuilt)?;

        for descriptor in &sorted {
            match descriptor.factory {
                Some(factory) => {
                    debug!(component = %descriptor.name, "constructing component");

                    let component = factory(&mut ctx).await?;

                    ctx.insert(component);

                    trace!(component = %descriptor.name, "component ready");
                }

                None => {
                    // Manually-provided: the instance must already be seeded.
                    if !ctx.contains((descriptor.ty.type_id)()) {
                        error!(component = %descriptor.name, "no instance provided for factory-less component");
                        return Err(Error::MissingComponent(descriptor.name));
                    }

                    trace!(component = %descriptor.name, "using provided instance");
                }
            }
        }

        let components = ctx.into_components();

        info!(count = components.len(), "container built");

        Ok(Self { components })
    }

    /// Returns a cloned `Arc<T>` for the registered component of type `T`.
    pub fn get<T: Any + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let type_id = TypeId::of::<T>();
        let component = self.components.get(&type_id)?;

        component.value.downcast_ref::<Arc<T>>().cloned()
    }
}

fn topological_sort<'a>(
    components: &'a [ComponentDescriptor],
    prebuilt: &HashSet<TypeId>,
) -> crate::Result<Vec<&'a ComponentDescriptor>> {
    trace!(total = components.len(), "starting topological sort");

    let mut result: Vec<&'a ComponentDescriptor> = Vec::new();
    let mut remaining: Vec<&'a ComponentDescriptor> = components.iter().collect();

    while !remaining.is_empty() {
        let before_len = remaining.len();

        remaining.retain(|descriptor| {
            let resolved = descriptor
                .dependencies
                .iter()
                .filter(|dep| !dep.optional)
                .all(|dep| {
                    let dep_type_id = (dep.ty.type_id)();
                    prebuilt.contains(&dep_type_id)
                        || result.iter().any(|r| (r.ty.type_id)() == dep_type_id)
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
            error!("dependency cycle detected in component graph");
            return Err(Error::DependencyCycle);
        }
    }

    trace!(count = result.len(), "topological sort complete");

    Ok(result)
}
