use std::any::TypeId;
use std::collections::{BTreeSet, HashMap, HashSet};

use overseerd_core::{DependencyDescriptor, ScopeId, Singleton, StaticScope};
use overseerd_di::{BoxedComponent, ComponentDescriptor};

use crate::scope::SeedDestination;

/// Immutable callback-free input for prepared-state tooling projection.
pub(crate) struct ProjectionSnapshot {
    components: Vec<ComponentSnapshot>,
    root_plan: Vec<ConstructionPlanEntry>,
    scope_plans: HashMap<ScopeId, Vec<ConstructionPlanEntry>>,
}

impl ProjectionSnapshot {
    pub(crate) fn capture(
        descriptors: &[ComponentDescriptor],
        root_order: &[ComponentDescriptor],
        scope_orders: &HashMap<ScopeId, Vec<ComponentDescriptor>>,
        instances: &[BoxedComponent],
        seed_destinations: &HashMap<TypeId, SeedDestination>,
    ) -> Self {
        let seeded: HashSet<_> = instances
            .iter()
            .map(|instance| instance.ty.type_id)
            .collect();
        let components: Vec<_> = descriptors
            .iter()
            .map(|descriptor| ComponentSnapshot::capture(*descriptor, &seeded, seed_destinations))
            .collect();
        let root_ids: BTreeSet<_> = root_order
            .iter()
            .map(|component| component.ty.type_id)
            .collect();
        let mut root_plan: Vec<_> = components
            .iter()
            .filter(|component| component.descriptor.scope.id() == Singleton::ID)
            .filter(|component| component.seeded)
            .filter(|component| !root_ids.contains(&component.descriptor.ty.type_id))
            .map(ConstructionPlanEntry::from_component)
            .collect();

        root_plan.extend(root_order.iter().map(|descriptor| {
            ConstructionPlanEntry::from_component(component(&components, descriptor.ty.type_id))
        }));

        let scope_plans = scope_orders
            .iter()
            .map(|(scope, order)| {
                let entries = order
                    .iter()
                    .map(|descriptor| {
                        ConstructionPlanEntry::from_component(component(
                            &components,
                            descriptor.ty.type_id,
                        ))
                    })
                    .collect();

                (*scope, entries)
            })
            .collect();

        Self {
            components,
            root_plan,
            scope_plans,
        }
    }

    pub(crate) fn components(&self) -> &[ComponentSnapshot] {
        &self.components
    }

    pub(crate) fn root_plan(&self) -> &[ConstructionPlanEntry] {
        &self.root_plan
    }

    pub(crate) fn scope_plan(&self, scope: &ScopeId) -> Option<&[ConstructionPlanEntry]> {
        self.scope_plans.get(scope).map(Vec::as_slice)
    }
}

/// Callback-derived facts for one effective component descriptor.
#[derive(Clone)]
pub(crate) struct ComponentSnapshot {
    pub(crate) descriptor: ComponentDescriptor,
    pub(crate) has_factory: bool,
    pub(crate) dependencies: Vec<DependencyDescriptor>,
    pub(crate) hooks: Vec<HookSnapshot>,
    pub(crate) seeded: bool,
    pub(crate) seed_destination: Option<ScopeId>,
}

impl ComponentSnapshot {
    fn capture(
        descriptor: ComponentDescriptor,
        seeded: &HashSet<TypeId>,
        seed_destinations: &HashMap<TypeId, SeedDestination>,
    ) -> Self {
        let factory = descriptor
            .effective_factory()
            .expect("validated component factory remains unambiguous");
        let dependencies = factory.map_or_else(Vec::new, |factory| (factory.dependencies)());
        let mut hooks: Vec<_> = (descriptor.hooks)()
            .iter()
            .copied()
            .map(HookSnapshot::capture)
            .collect();

        hooks.sort_by(|left, right| {
            left.kind
                .cmp(right.kind)
                .then_with(|| left.ordinal.cmp(&right.ordinal))
        });

        Self {
            descriptor,
            has_factory: factory.is_some(),
            dependencies,
            hooks,
            seeded: seeded.contains(&descriptor.ty.type_id),
            seed_destination: seed_destinations
                .get(&descriptor.ty.type_id)
                .map(|seed| seed.scope),
        }
    }
}

/// Callback-derived facts for one retained hook descriptor.
#[derive(Clone)]
pub(crate) struct HookSnapshot {
    pub(crate) ordinal: u32,
    pub(crate) kind: &'static str,
    pub(crate) dependencies: Vec<DependencyDescriptor>,
}

impl HookSnapshot {
    fn capture(descriptor: overseerd_hooks::HookDescriptor) -> Self {
        Self {
            ordinal: descriptor.ordinal,
            kind: descriptor.kind,
            dependencies: (descriptor.dependencies)(),
        }
    }
}

/// One exact prepared construction-plan entry with its snapshotted selection outcome.
#[derive(Clone, Copy)]
pub(crate) struct ConstructionPlanEntry {
    pub(crate) descriptor: ComponentDescriptor,
    pub(crate) has_factory: bool,
}

impl ConstructionPlanEntry {
    fn from_component(component: &ComponentSnapshot) -> Self {
        Self {
            descriptor: component.descriptor,
            has_factory: component.has_factory,
        }
    }
}

fn component(components: &[ComponentSnapshot], ty: TypeId) -> &ComponentSnapshot {
    components
        .iter()
        .find(|component| component.descriptor.ty.type_id == ty)
        .expect("prepared construction plan references an effective component")
}
