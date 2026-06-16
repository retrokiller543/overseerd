use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

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
    pub async fn build(registry: &Registry) -> crate::Result<Self> {
        let sorted = topological_sort(&registry.components)?;
        let mut ctx = ComponentConstructionContext::new();

        for descriptor in sorted {
            let component = (descriptor.factory)(&mut ctx).await?;
            ctx.insert(component);
        }

        Ok(Self {
            components: ctx.into_components(),
        })
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
                result.push(descriptor);
                false
            } else {
                true
            }
        });

        if remaining.len() == before_len {
            return Err(Error::DependencyCycle);
        }
    }

    Ok(result)
}
