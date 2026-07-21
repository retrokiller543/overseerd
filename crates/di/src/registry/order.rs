use std::any::TypeId;
use std::collections::{HashMap, HashSet};

use crate::descriptors::{ComponentDescriptor, ProviderDescriptor, ProviderOrderDirection};
use crate::error::Error;

pub(super) fn build(
    components: &[ComponentDescriptor],
    providers: &[ProviderDescriptor],
) -> crate::Result<HashMap<TypeId, HashMap<TypeId, usize>>> {
    let component_ids: HashSet<_> = components.iter().map(|c| c.ty.type_id).collect();
    let mut by_concrete: HashMap<TypeId, Vec<&ProviderDescriptor>> = HashMap::new();
    let mut by_trait: HashMap<TypeId, Vec<&ProviderDescriptor>> = HashMap::new();

    for provider in providers {
        by_concrete
            .entry(provider.concrete_ty.type_id)
            .or_default()
            .push(provider);
        by_trait
            .entry(provider.trait_ty.type_id)
            .or_default()
            .push(provider);
    }

    for source_providers in by_concrete.values() {
        let source = source_providers[0];

        for ordering in source.ordering {
            let target_id = ordering.target.type_id;

            if !component_ids.contains(&target_id) {
                return Err(Error::MissingProviderOrderTarget {
                    component: (source.concrete_ty.type_name)().to_string(),
                    target: (ordering.target.type_name)().to_string(),
                });
            }

            if target_id == source.concrete_ty.type_id {
                return Err(Error::SelfProviderOrder {
                    component: (source.concrete_ty.type_name)().to_string(),
                });
            }

            for trait_ty in ordering.traits {
                let trait_id = trait_ty.type_id;

                if !source_providers
                    .iter()
                    .any(|p| p.trait_ty.type_id == trait_id)
                {
                    return Err(Error::ProviderOrderSourceTraitMismatch {
                        component: (source.concrete_ty.type_name)().to_string(),
                        trait_name: (trait_ty.type_name)().to_string(),
                    });
                }
            }
        }
    }

    let mut plan = HashMap::new();

    for (trait_id, trait_providers) in by_trait {
        let mut edges: HashMap<TypeId, HashSet<TypeId>> = HashMap::new();
        let mut indegree: HashMap<TypeId, usize> = trait_providers
            .iter()
            .map(|p| (p.concrete_ty.type_id, 0))
            .collect();

        for source in &trait_providers {
            let source_id = source.concrete_ty.type_id;

            for ordering in source.ordering {
                if !ordering.traits.is_empty()
                    && !ordering.traits.iter().any(|ty| ty.type_id == trait_id)
                {
                    continue;
                }

                let target_id = ordering.target.type_id;

                if !trait_providers
                    .iter()
                    .any(|p| p.concrete_ty.type_id == target_id)
                {
                    if ordering.traits.is_empty() {
                        continue;
                    }

                    return Err(Error::ProviderOrderTargetTraitMismatch {
                        component: (source.concrete_ty.type_name)().to_string(),
                        target: (ordering.target.type_name)().to_string(),
                        trait_name: (source.trait_ty.type_name)().to_string(),
                    });
                }

                let (from, to) = match ordering.direction {
                    ProviderOrderDirection::Before => (source_id, target_id),
                    ProviderOrderDirection::After => (target_id, source_id),
                };

                if edges.entry(from).or_default().insert(to) {
                    *indegree.entry(to).or_default() += 1;
                }
            }
        }

        let mut ordered = Vec::with_capacity(trait_providers.len());

        while ordered.len() < trait_providers.len() {
            let next = trait_providers
                .iter()
                .filter(|provider| {
                    let id = provider.concrete_ty.type_id;

                    indegree.get(&id) == Some(&0) && !ordered.contains(&id)
                })
                .min_by(|left, right| {
                    left.priority
                        .cmp(&right.priority)
                        .then_with(|| {
                            (left.concrete_ty.type_name)().cmp((right.concrete_ty.type_name)())
                        })
                        .then_with(|| left.qualifier.cmp(right.qualifier))
                });
            let Some(next) = next else {
                let components = trait_providers
                    .iter()
                    .filter(|provider| !ordered.contains(&provider.concrete_ty.type_id))
                    .map(|provider| (provider.concrete_ty.type_name)())
                    .collect::<Vec<_>>()
                    .join(", ");

                return Err(Error::ProviderOrderCycle {
                    trait_name: (trait_providers[0].trait_ty.type_name)().to_string(),
                    components,
                });
            };
            let next_id = next.concrete_ty.type_id;

            ordered.push(next_id);

            if let Some(successors) = edges.get(&next_id) {
                for successor in successors {
                    *indegree
                        .get_mut(successor)
                        .expect("provider indegree exists") -= 1;
                }
            }
        }

        plan.insert(
            trait_id,
            ordered
                .into_iter()
                .enumerate()
                .map(|(i, id)| (id, i))
                .collect(),
        );
    }

    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::{BoxedComponent, ProviderOrder};
    use overseerd_core::{Singleton, TypeDescriptor};

    trait Trait: Send + Sync {}
    trait OtherTrait: Send + Sync {}
    struct Alpha;
    struct Beta;
    struct Gamma;

    fn erase(_: &BoxedComponent) -> BoxedComponent {
        panic!("ordering tests do not construct providers")
    }

    fn component<T: 'static>(name: &'static str) -> ComponentDescriptor {
        ComponentDescriptor::manual(name, name, TypeDescriptor::of::<T>(name), &Singleton)
    }

    fn provider<T: 'static>(
        name: &'static str,
        qualifier: &'static str,
        ordering: &'static [ProviderOrder],
    ) -> ProviderDescriptor {
        provider_as::<T, dyn Trait>(name, qualifier, ordering)
    }

    fn provider_as<T: 'static, P: ?Sized + 'static>(
        name: &'static str,
        qualifier: &'static str,
        ordering: &'static [ProviderOrder],
    ) -> ProviderDescriptor {
        ProviderDescriptor {
            trait_ty: TypeDescriptor::of::<P>("provider trait"),
            concrete_ty: TypeDescriptor::of::<T>(name),
            qualifier,
            primary: false,
            priority: 0,
            ordering,
            erase,
        }
    }

    #[test]
    fn unconstrained_order_uses_type_name_then_qualifier() {
        let components = [component::<Beta>("Beta"), component::<Alpha>("Alpha")];
        let providers = [
            provider::<Beta>("Beta", "a", &[]),
            provider::<Alpha>("Alpha", "z", &[]),
        ];
        let plan = build(&components, &providers).expect("provider plan");
        let order = &plan[&TypeId::of::<dyn Trait>()];

        assert_eq!(order[&TypeId::of::<Alpha>()], 0);
        assert_eq!(order[&TypeId::of::<Beta>()], 1);
    }

    #[test]
    fn unconstrained_order_uses_priority_before_type_name() {
        let components = [component::<Alpha>("Alpha"), component::<Beta>("Beta")];
        let mut alpha = provider::<Alpha>("Alpha", "alpha", &[]);
        let mut beta = provider::<Beta>("Beta", "beta", &[]);

        alpha.priority = 20;
        beta.priority = -10;

        let plan = build(&components, &[alpha, beta]).expect("provider plan");
        let order = &plan[&TypeId::of::<dyn Trait>()];

        assert_eq!(order[&TypeId::of::<Beta>()], 0);
        assert_eq!(order[&TypeId::of::<Alpha>()], 1);
    }

    #[test]
    fn constraints_form_a_global_topological_order() {
        static BETWEEN: [ProviderOrder; 2] = [
            ProviderOrder {
                target: TypeDescriptor::of::<Alpha>("Alpha"),
                traits: &[],
                direction: ProviderOrderDirection::After,
            },
            ProviderOrder {
                target: TypeDescriptor::of::<Beta>("Beta"),
                traits: &[TypeDescriptor::of::<dyn Trait>("dyn Trait")],
                direction: ProviderOrderDirection::Before,
            },
        ];
        let components = [
            component::<Alpha>("Alpha"),
            component::<Beta>("Beta"),
            component::<Gamma>("Gamma"),
        ];
        let providers = [
            provider::<Beta>("Beta", "beta", &[]),
            provider::<Gamma>("Gamma", "gamma", &BETWEEN),
            provider::<Alpha>("Alpha", "alpha", &[]),
        ];
        let plan = build(&components, &providers).expect("provider plan");
        let order = &plan[&TypeId::of::<dyn Trait>()];

        assert_eq!(order[&TypeId::of::<Alpha>()], 0);
        assert_eq!(order[&TypeId::of::<Gamma>()], 1);
        assert_eq!(order[&TypeId::of::<Beta>()], 2);
    }

    #[test]
    fn priority_orders_providers_that_share_an_after_constraint() {
        static AFTER_ALPHA: [ProviderOrder; 1] = [ProviderOrder {
            target: TypeDescriptor::of::<Alpha>("Alpha"),
            traits: &[],
            direction: ProviderOrderDirection::After,
        }];
        let components = [
            component::<Alpha>("Alpha"),
            component::<Beta>("Beta"),
            component::<Gamma>("Gamma"),
        ];
        let alpha = provider::<Alpha>("Alpha", "alpha", &[]);
        let mut beta = provider::<Beta>("Beta", "beta", &AFTER_ALPHA);
        let mut gamma = provider::<Gamma>("Gamma", "gamma", &AFTER_ALPHA);

        beta.priority = 20;
        gamma.priority = -20;

        let plan = build(&components, &[beta, alpha, gamma]).expect("provider plan");
        let order = &plan[&TypeId::of::<dyn Trait>()];

        assert_eq!(order[&TypeId::of::<Alpha>()], 0);
        assert_eq!(order[&TypeId::of::<Gamma>()], 1);
        assert_eq!(order[&TypeId::of::<Beta>()], 2);
    }

    #[test]
    fn relative_constraints_take_precedence_over_priority() {
        static BETA_AFTER_ALPHA: [ProviderOrder; 1] = [ProviderOrder {
            target: TypeDescriptor::of::<Alpha>("Alpha"),
            traits: &[],
            direction: ProviderOrderDirection::After,
        }];
        let components = [component::<Alpha>("Alpha"), component::<Beta>("Beta")];
        let mut alpha = provider::<Alpha>("Alpha", "alpha", &[]);
        let mut beta = provider::<Beta>("Beta", "beta", &BETA_AFTER_ALPHA);

        alpha.priority = 100;
        beta.priority = -100;

        let plan = build(&components, &[beta, alpha]).expect("provider plan");
        let order = &plan[&TypeId::of::<dyn Trait>()];

        assert_eq!(order[&TypeId::of::<Alpha>()], 0);
        assert_eq!(order[&TypeId::of::<Beta>()], 1);
    }

    #[test]
    fn unrestricted_ordering_ignores_traits_not_shared_with_the_target() {
        static ALPHA_AFTER_BETA: [ProviderOrder; 1] = [ProviderOrder {
            target: TypeDescriptor::of::<Beta>("Beta"),
            traits: &[],
            direction: ProviderOrderDirection::After,
        }];
        let components = [component::<Alpha>("Alpha"), component::<Beta>("Beta")];
        let providers = [
            provider::<Alpha>("Alpha", "alpha", &ALPHA_AFTER_BETA),
            provider_as::<Beta, dyn OtherTrait>("Beta", "beta", &[]),
        ];
        let plan = build(&components, &providers).expect("unshared traits are ignored");

        assert_eq!(plan[&TypeId::of::<dyn Trait>()][&TypeId::of::<Alpha>()], 0);
        assert_eq!(
            plan[&TypeId::of::<dyn OtherTrait>()][&TypeId::of::<Beta>()],
            0
        );
    }

    #[test]
    fn restricted_ordering_requires_the_target_to_provide_the_trait() {
        static ALPHA_AFTER_BETA_AS_TRAIT: [ProviderOrder; 1] = [ProviderOrder {
            target: TypeDescriptor::of::<Beta>("Beta"),
            traits: &[TypeDescriptor::of::<dyn Trait>("dyn Trait")],
            direction: ProviderOrderDirection::After,
        }];
        let components = [component::<Alpha>("Alpha"), component::<Beta>("Beta")];
        let providers = [
            provider::<Alpha>("Alpha", "alpha", &ALPHA_AFTER_BETA_AS_TRAIT),
            provider_as::<Beta, dyn OtherTrait>("Beta", "beta", &[]),
        ];

        assert!(matches!(
            build(&components, &providers),
            Err(Error::ProviderOrderTargetTraitMismatch { .. })
        ));
    }

    #[test]
    fn reports_missing_self_mismatch_and_cycle_errors() {
        static MISSING: [ProviderOrder; 1] = [ProviderOrder {
            target: TypeDescriptor::of::<Gamma>("Gamma"),
            traits: &[],
            direction: ProviderOrderDirection::Before,
        }];
        static SELF: [ProviderOrder; 1] = [ProviderOrder {
            target: TypeDescriptor::of::<Alpha>("Alpha"),
            traits: &[],
            direction: ProviderOrderDirection::Before,
        }];
        static ALPHA_BEFORE_BETA: [ProviderOrder; 1] = [ProviderOrder {
            target: TypeDescriptor::of::<Beta>("Beta"),
            traits: &[],
            direction: ProviderOrderDirection::Before,
        }];
        static BETA_BEFORE_ALPHA: [ProviderOrder; 1] = [ProviderOrder {
            target: TypeDescriptor::of::<Alpha>("Alpha"),
            traits: &[],
            direction: ProviderOrderDirection::Before,
        }];
        let alpha = component::<Alpha>("Alpha");
        let beta = component::<Beta>("Beta");

        assert!(matches!(
            build(&[alpha], &[provider::<Alpha>("Alpha", "alpha", &MISSING)]),
            Err(Error::MissingProviderOrderTarget { .. })
        ));
        assert!(matches!(
            build(&[alpha], &[provider::<Alpha>("Alpha", "alpha", &SELF)]),
            Err(Error::SelfProviderOrder { .. })
        ));
        assert!(matches!(
            build(
                &[alpha, beta],
                &[
                    provider::<Alpha>("Alpha", "alpha", &ALPHA_BEFORE_BETA),
                    provider::<Beta>("Beta", "beta", &BETA_BEFORE_ALPHA),
                ],
            ),
            Err(Error::ProviderOrderCycle { .. })
        ));
    }
}
