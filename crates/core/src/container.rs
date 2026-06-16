use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

use tracing::{debug, error, info, instrument, trace};

use crate::{
    descriptors::{component::ComponentConstructionContext, ComponentDescriptor},
    Error, Registry,
};

/// Holds all fully constructed component instances for a running daemon.
pub struct Container {
    components: HashMap<TypeId, crate::BoxedComponent>,
}

impl Container {
    /// Resolves all registered components in dependency order and returns a built Container.
    #[instrument(skip_all, fields(count = registry.components.len()))]
    pub async fn build(registry: &Registry) -> crate::Result<Self> {
        debug!("resolving component dependency order");

        let sorted = topological_sort(&registry.components)?;
        let mut ctx = ComponentConstructionContext::new();

        for descriptor in &sorted {
            debug!(component = %descriptor.name, "constructing component");

            let component = (descriptor.factory)(&mut ctx).await?;

            ctx.insert(component);

            trace!(component = %descriptor.name, "component ready");
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

fn topological_sort(
    components: &[&'static ComponentDescriptor],
) -> crate::Result<Vec<&'static ComponentDescriptor>> {
    trace!(total = components.len(), "starting topological sort");

    let mut result: Vec<&'static ComponentDescriptor> = Vec::new();
    let mut remaining: Vec<&'static ComponentDescriptor> = components.to_vec();

    while !remaining.is_empty() {
        let before_len = remaining.len();

        remaining.retain(|descriptor| {
            let resolved = descriptor
                .dependencies
                .iter()
                .filter(|dep| !dep.optional)
                .all(|dep| {
                    let dep_type_id = (dep.ty.type_id)();
                    result.iter().any(|r| (r.ty.type_id)() == dep_type_id)
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
