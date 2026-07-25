use std::any::TypeId;
use std::collections::{HashMap, HashSet};

use overseerd_core::{ScopeId, Singleton, StaticScope, Transient};
use overseerd_di::{ComponentDescriptor, ProviderDescriptor, topological_sort};

use super::PreparedScopeTopology;

/// The validated construction plan for root, transient, and protocol-owned scopes.
#[derive(Debug)]
pub(crate) struct ScopePlan {
    pub(crate) singletons: Vec<ComponentDescriptor>,
    pub(crate) transient: HashMap<TypeId, ComponentDescriptor>,
    pub(crate) orders: HashMap<ScopeId, Vec<ComponentDescriptor>>,
    pub(crate) seed_destinations: HashMap<TypeId, SeedDestination>,
}

/// The declared destination and diagnostic name of a factory-less scoped component.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SeedDestination {
    pub(crate) scope: ScopeId,
    pub(crate) type_name: &'static str,
}

impl ScopePlan {
    /// Partitions resolved descriptors and computes each boundary's local factory order.
    pub(crate) fn partition(
        resolved: &[ComponentDescriptor],
        providers: &[ProviderDescriptor],
        topology: &PreparedScopeTopology,
    ) -> crate::Result<Self> {
        let mut singletons = Vec::new();
        let mut transient = HashMap::new();
        let mut by_scope: HashMap<ScopeId, Vec<ComponentDescriptor>> = HashMap::new();
        let mut descriptors_by_scope: HashMap<ScopeId, Vec<ComponentDescriptor>> = HashMap::new();
        let mut factoryless_by_scope: HashMap<ScopeId, HashSet<TypeId>> = HashMap::new();
        let mut seed_destinations = HashMap::new();

        for component in resolved {
            let scope = component.scope.id();

            if scope == <Transient as StaticScope>::ID {
                transient.insert(component.ty.type_id, *component);

                continue;
            }

            if scope == <Singleton as StaticScope>::ID {
                singletons.push(*component);

                continue;
            }

            if !topology.contains(scope) {
                return Err(crate::Error::UndeclaredScope {
                    component: (component.ty.type_name)().to_string(),
                    scope,
                });
            }

            descriptors_by_scope
                .entry(scope)
                .or_default()
                .push(*component);

            if component.effective_factory()?.is_some() {
                by_scope.entry(scope).or_default().push(*component);
            } else {
                factoryless_by_scope
                    .entry(scope)
                    .or_default()
                    .insert(component.ty.type_id);
                seed_destinations.insert(
                    component.ty.type_id,
                    SeedDestination {
                        scope,
                        type_name: (component.ty.type_name)(),
                    },
                );
            }
        }

        let root: HashSet<TypeId> = singletons
            .iter()
            .map(|component| component.ty.type_id)
            .collect();
        let mut orders = HashMap::new();

        for boundary in topology.boundaries() {
            let scope = boundary.id();
            let mut prebuilt = root.clone();

            for ancestor in topology.ancestors(scope) {
                if ancestor == <Singleton as StaticScope>::ID {
                    continue;
                }

                prebuilt.extend(
                    descriptors_by_scope
                        .get(&ancestor)
                        .into_iter()
                        .flatten()
                        .map(|component| component.ty.type_id),
                );
            }

            prebuilt.extend(
                factoryless_by_scope
                    .get(&scope)
                    .into_iter()
                    .flatten()
                    .copied(),
            );

            let local = by_scope.get(&scope).map_or(&[][..], Vec::as_slice);
            let order = topological_sort(local, &prebuilt, providers, &transient)?
                .into_iter()
                .copied()
                .collect();

            orders.insert(scope, order);
        }

        Ok(Self {
            singletons,
            transient,
            orders,
            seed_destinations,
        })
    }
}

#[cfg(test)]
mod tests;
