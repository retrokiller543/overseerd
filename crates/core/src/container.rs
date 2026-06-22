use std::{
    any::TypeId,
    collections::{HashMap, HashSet},
};

use tracing::{debug, error, info, instrument, trace};

use crate::{
    BoxedComponent, Error,
    descriptors::{
        Cardinality, Component, ComponentDescriptor, ProviderDescriptor,
        component::ComponentConstructionContext,
    },
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
        providers: &[ProviderDescriptor],
    ) -> crate::Result<Self> {
        debug!("resolving component dependency order");

        let mut ctx = ComponentConstructionContext::new();
        let mut prebuilt: HashSet<TypeId> = HashSet::new();

        for component in instances {
            let type_id = (component.ty.type_id)();

            prebuilt.insert(type_id);
            ctx.insert(component);
            register_providers_for(&mut ctx, providers, type_id);
        }

        let sorted = topological_sort(components, &prebuilt, providers)?;

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

            // Alias the just-built instance under each trait it provides. This is
            // a clone of the existing `Arc` re-typed as `Arc<dyn Trait>`, never a
            // second construction.
            register_providers_for(&mut ctx, providers, (descriptor.ty.type_id)());
        }

        let components = ctx.into_components();

        info!(count = components.len(), "container built");

        Ok(Self { components })
    }

    /// Returns the registered component of type `T` as its handle (`Arc<T>` by
    /// default, or the by-value handle for a `#[component(by_value)]` type).
    pub fn get<T: Component>(&self) -> Option<T::Handle> {
        let type_id = TypeId::of::<T>();
        let component = self.components.get(&type_id)?;

        component.value.downcast_ref::<T::Handle>().cloned()
    }
}

/// Registers every provider declared by the just-built concrete `concrete_id`,
/// aliasing its single instance under each trait it provides.
fn register_providers_for(
    ctx: &mut ComponentConstructionContext,
    providers: &[ProviderDescriptor],
    concrete_id: TypeId,
) {
    for provider in providers
        .iter()
        .filter(|p| (p.concrete_ty.type_id)() == concrete_id)
    {
        ctx.register_provider(provider);
    }
}

fn topological_sort<'a>(
    components: &'a [ComponentDescriptor],
    prebuilt: &HashSet<TypeId>,
    providers: &[ProviderDescriptor],
) -> crate::Result<Vec<&'a ComponentDescriptor>> {
    trace!(total = components.len(), "starting topological sort");

    // trait TypeId -> the concrete TypeIds that provide it. A dependency on a
    // trait must wait for all of its providers to be built.
    let mut provider_concretes: HashMap<TypeId, Vec<TypeId>> = HashMap::new();

    for provider in providers {
        provider_concretes
            .entry((provider.trait_ty.type_id)())
            .or_default()
            .push((provider.concrete_ty.type_id)());
    }

    let mut result: Vec<&'a ComponentDescriptor> = Vec::new();
    let mut remaining: Vec<&'a ComponentDescriptor> = components.iter().collect();

    while !remaining.is_empty() {
        let before_len = remaining.len();

        remaining.retain(|descriptor| {
            let is_built = |type_id: TypeId| {
                prebuilt.contains(&type_id) || result.iter().any(|r| (r.ty.type_id)() == type_id)
            };

            let resolved = descriptor
                .dependencies
                .iter()
                // `optional`/`dynamic` edges impose no build-ordering constraint.
                .filter(|dep| !dep.optional && !dep.dynamic)
                .all(|dep| dep_ready(dep.cardinality, (dep.ty.type_id)(), &provider_concretes, &is_built));

            if resolved {
                trace!(component = %descriptor.name, "dependency order resolved");
                result.push(descriptor);
                false
            } else {
                true
            }
        });

        if remaining.len() == before_len {
            let stuck = remaining
                .iter()
                .map(|d| d.name)
                .collect::<Vec<_>>()
                .join(", ");

            error!(components = %stuck, "dependency cycle detected in component graph");

            return Err(Error::DependencyCycle(stuck));
        }
    }

    trace!(count = result.len(), "topological sort complete");

    Ok(result)
}

/// Whether a dependency's predecessors are all built. A trait edge waits for
/// every provider of that trait; a single concrete edge waits for that concrete;
/// a multi-valued edge with no providers is trivially ready (empty is valid).
fn dep_ready(
    cardinality: Cardinality,
    dep_type_id: TypeId,
    provider_concretes: &HashMap<TypeId, Vec<TypeId>>,
    is_built: &impl Fn(TypeId) -> bool,
) -> bool {
    if let Some(concretes) = provider_concretes.get(&dep_type_id) {
        return concretes.iter().all(|id| is_built(*id));
    }

    match cardinality {
        Cardinality::One => is_built(dep_type_id),
        Cardinality::Collection | Cardinality::Keyed => true,
    }
}
