use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use upwell_core::{
    ConditionDescriptor, ConditionPredicate, ConditionPredicateKind, ConditionScalar,
    ConditionScalarKind, ConfigFactDescriptor, ConfigFactId, DescriptorSource, ProviderMappingId,
    StaticScope,
};

use crate::{
    ComponentDescriptor, ComponentRegistry, ProviderDescriptor, ProviderSelectionModel, Singleton,
};

/// Typed config facts supplied to one catalog evaluation.
#[derive(Clone, Default)]
pub struct ConditionFactSnapshot {
    facts: BTreeMap<ConfigFactId, ConditionScalar>,
}

impl ConditionFactSnapshot {
    pub fn new(
        facts: impl IntoIterator<Item = (ConfigFactId, ConditionScalar)>,
    ) -> Result<Self, ConditionError> {
        let mut snapshot = Self::default();

        for (id, value) in facts {
            if snapshot.facts.insert(id, value).is_some() {
                return Err(ConditionError::DuplicateFactValue(id));
            }
        }

        Ok(snapshot)
    }
}

impl fmt::Debug for ConditionFactSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let facts = self
            .facts
            .iter()
            .map(|(id, value)| (*id, value.kind()))
            .collect::<Vec<_>>();

        formatter
            .debug_struct("ConditionFactSnapshot")
            .field("facts", &facts)
            .finish()
    }
}

/// One redacted condition-node outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConditionDecision {
    pub component_id: &'static str,
    pub condition_id: &'static str,
    pub predicate: ConditionPredicateKind,
    pub source: DescriptorSource,
    pub outcome: bool,
}

/// One fact whose change can invalidate a component's condition decision.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ConditionDependency {
    Config(ConfigFactId),
    Component(&'static str),
    Provider(ProviderMappingId),
}

/// Deterministic eligibility result for one supplied fact snapshot.
#[derive(Clone, Debug)]
pub struct ConditionEvaluation {
    eligible: ComponentRegistry,
    components: BTreeMap<&'static str, bool>,
    providers: BTreeMap<ProviderMappingId, bool>,
    decisions: Vec<ConditionDecision>,
}

/// Eligibility decisions whose ordinary DI graph has also passed validation.
pub struct ValidatedConditionEvaluation {
    evaluation: ConditionEvaluation,
    selection: ProviderSelectionModel,
}

impl ValidatedConditionEvaluation {
    pub fn evaluation(&self) -> &ConditionEvaluation {
        &self.evaluation
    }

    pub fn registry(&self) -> &ComponentRegistry {
        self.evaluation.eligible_registry()
    }

    pub fn provider_selection(&self) -> &ProviderSelectionModel {
        &self.selection
    }
}

impl ConditionEvaluation {
    pub fn eligible_registry(&self) -> &ComponentRegistry {
        &self.eligible
    }

    pub fn component_eligible(&self, id: &str) -> Option<bool> {
        self.components.get(id).copied()
    }

    pub fn provider_eligible(&self, id: ProviderMappingId) -> Option<bool> {
        self.providers.get(&id).copied()
    }

    pub fn decisions(&self) -> &[ConditionDecision] {
        &self.decisions
    }
}

/// Validated static inputs for deterministic condition evaluation.
pub struct ConditionCatalog {
    components: BTreeMap<&'static str, ComponentDescriptor>,
    component_order: Vec<&'static str>,
    providers: Vec<(ProviderMappingId, ProviderDescriptor)>,
    facts: BTreeMap<ConfigFactId, ConfigFactDescriptor>,
    provider_order: HashMap<TypeId, HashMap<TypeId, usize>>,
}

impl ConditionCatalog {
    pub fn new(
        registry: &ComponentRegistry,
        config_facts: impl IntoIterator<Item = ConfigFactDescriptor>,
    ) -> Result<Self, ConditionError> {
        validate_manual_overrides(&registry.components)?;

        let resolved = registry
            .resolved_components()
            .map_err(ConditionError::Registry)?;
        let mut components = BTreeMap::new();
        let mut by_type = HashMap::new();

        for component in resolved {
            if components.insert(component.id, component).is_some() {
                return Err(ConditionError::DuplicateComponentId(component.id));
            }

            by_type.insert(component.ty.type_id, component);
        }

        let mut facts = BTreeMap::new();

        for fact in config_facts {
            if facts.insert(fact.id, fact).is_some() {
                return Err(ConditionError::DuplicateFactDescriptor(fact.id));
            }
        }

        let mut providers = Vec::with_capacity(registry.providers.len());
        let mut provider_ids = BTreeSet::new();

        for provider in &registry.providers {
            let component = by_type.get(&provider.concrete_ty.type_id).ok_or(
                ConditionError::MissingProviderComponent {
                    trait_type: (provider.trait_ty.type_name)(),
                    qualifier: provider.qualifier,
                },
            )?;
            let id = provider.mapping_id(component);

            if !provider_ids.insert(id) {
                return Err(ConditionError::DuplicateProviderMapping(id));
            }

            providers.push((id, *provider));
        }

        providers.sort_by_key(|(id, _)| *id);

        let provider_order = registry
            .provider_order(&components.values().copied().collect::<Vec<_>>())
            .map_err(ConditionError::Registry)?;
        let catalog = Self {
            component_order: components.keys().copied().collect(),
            components,
            providers,
            facts,
            provider_order,
        };

        catalog.validate_conditions()?;
        catalog.validate_acyclic()?;

        Ok(catalog)
    }

    pub fn evaluate(
        &self,
        snapshot: &ConditionFactSnapshot,
    ) -> Result<ConditionEvaluation, ConditionError> {
        self.validate_snapshot(snapshot)?;

        let mut states = BTreeMap::new();
        let mut decisions = Vec::new();

        for component in &self.component_order {
            self.evaluate_component(component, snapshot, &mut states, &mut decisions)?;
        }

        let providers = self
            .providers
            .iter()
            .map(|(id, _)| (*id, states[id.component]))
            .collect::<BTreeMap<_, _>>();
        let eligible = ComponentRegistry {
            components: self
                .component_order
                .iter()
                .filter(|id| states[**id])
                .map(|id| self.components[id])
                .collect(),
            providers: self
                .providers
                .iter()
                .filter(|(id, _)| states[id.component])
                .map(|(_, provider)| *provider)
                .collect(),
        };

        Ok(ConditionEvaluation {
            eligible,
            components: states,
            providers,
            decisions,
        })
    }

    /// Evaluates eligibility and runs the existing rank-based DI graph validation.
    pub fn evaluate_validated(
        &self,
        snapshot: &ConditionFactSnapshot,
    ) -> Result<ValidatedConditionEvaluation, ConditionError> {
        let evaluation = self.evaluate(snapshot)?;
        let selection = ProviderSelectionModel::new(
            &evaluation.eligible.components,
            evaluation.eligible.providers.clone(),
            self.provider_order.clone(),
        )
        .map_err(ConditionError::Registry)?;

        evaluation
            .eligible_registry()
            .validate_with_scope_access(
                &evaluation.eligible.components,
                &selection,
                |consumer, dependency| {
                    if dependency.is_transient() {
                        return true;
                    }

                    if consumer.is_transient() {
                        return dependency.id() == Singleton::ID;
                    }

                    dependency.rank() >= consumer.rank()
                },
            )
            .map_err(ConditionError::Registry)?;

        Ok(ValidatedConditionEvaluation {
            evaluation,
            selection,
        })
    }

    /// Returns the stable facts that can invalidate one component's eligibility.
    pub fn dependencies(
        &self,
        component_id: &str,
    ) -> Result<Vec<ConditionDependency>, ConditionError> {
        let component = self
            .components
            .get(component_id)
            .ok_or(ConditionError::UnknownComponentId(component_id.to_string()))?;
        let mut dependencies = BTreeSet::new();

        if let Some(root) = component.condition {
            collect_dependencies(root, &mut dependencies);
        }

        Ok(dependencies.into_iter().collect())
    }

    fn validate_conditions(&self) -> Result<(), ConditionError> {
        for (owner, component) in &self.components {
            let Some(root) = component.condition else {
                continue;
            };

            if component.scope.id() != Singleton::ID {
                return Err(ConditionError::UnsupportedScope {
                    component_id: owner,
                    scope: component.scope.id(),
                });
            }

            if component
                .effective_factory()
                .map_err(ConditionError::Registry)?
                .is_none()
            {
                return Err(ConditionError::ConditionalManualComponent(owner));
            }

            let mut condition_ids = BTreeSet::new();

            self.validate_node(owner, root, &mut condition_ids)?;
        }

        Ok(())
    }

    fn validate_node(
        &self,
        owner: &'static str,
        node: &'static ConditionDescriptor,
        condition_ids: &mut BTreeSet<&'static str>,
    ) -> Result<(), ConditionError> {
        if !condition_ids.insert(node.id) {
            return Err(ConditionError::DuplicateConditionId {
                component_id: owner,
                condition_id: node.id,
            });
        }

        match node.predicate {
            ConditionPredicate::ConfigBool(fact) => {
                self.require_fact(fact, ConditionScalarKind::Bool, owner, node.id)?;
            }
            ConditionPredicate::ConfigEquals { fact, expected } => {
                self.require_fact(fact, expected.kind(), owner, node.id)?;
            }
            ConditionPredicate::ComponentEligible(component) => {
                if !self.components.contains_key(component) {
                    return Err(ConditionError::MissingComponentReference {
                        component_id: owner,
                        condition_id: node.id,
                        referenced: component,
                    });
                }
            }
            ConditionPredicate::ProviderMappingEligible(provider) => {
                if !self.providers.iter().any(|(id, _)| *id == provider) {
                    return Err(ConditionError::MissingProviderReference {
                        component_id: owner,
                        condition_id: node.id,
                        provider,
                    });
                }
            }
            ConditionPredicate::All(children) | ConditionPredicate::Any(children) => {
                let mut children = children.iter().collect::<Vec<_>>();
                children.sort_by_key(|child| child.id);

                for child in children {
                    self.validate_node(owner, child, condition_ids)?;
                }
            }
            ConditionPredicate::Not(child) => {
                self.validate_node(owner, child, condition_ids)?;
            }
        }

        Ok(())
    }

    fn require_fact(
        &self,
        fact: ConfigFactId,
        expected: ConditionScalarKind,
        component_id: &'static str,
        condition_id: &'static str,
    ) -> Result<(), ConditionError> {
        let descriptor = self
            .facts
            .get(&fact)
            .ok_or(ConditionError::MissingFactReference {
                component_id,
                condition_id,
                fact,
            })?;

        if descriptor.kind != expected {
            return Err(ConditionError::FactKindMismatch {
                fact,
                expected,
                actual: descriptor.kind,
            });
        }

        Ok(())
    }

    fn validate_snapshot(&self, snapshot: &ConditionFactSnapshot) -> Result<(), ConditionError> {
        for (id, descriptor) in &self.facts {
            let value = snapshot
                .facts
                .get(id)
                .ok_or(ConditionError::MissingFactValue(*id))?;

            if value.kind() != descriptor.kind {
                return Err(ConditionError::FactKindMismatch {
                    fact: *id,
                    expected: descriptor.kind,
                    actual: value.kind(),
                });
            }
        }

        for id in snapshot.facts.keys() {
            if !self.facts.contains_key(id) {
                return Err(ConditionError::UnknownFactValue(*id));
            }
        }

        Ok(())
    }

    fn validate_acyclic(&self) -> Result<(), ConditionError> {
        let graph = self.availability_graph();
        let mut completed = BTreeSet::new();

        for component in &self.component_order {
            let mut visiting = Vec::new();
            let mut active = BTreeSet::new();

            if let Some(cycle) = find_cycle(
                component,
                &graph,
                &mut visiting,
                &mut active,
                &mut completed,
            ) {
                return Err(ConditionError::AvailabilityCycle(cycle));
            }
        }

        Ok(())
    }

    fn availability_graph(&self) -> BTreeMap<&'static str, Vec<AvailabilityEdge>> {
        let mut graph = self
            .component_order
            .iter()
            .map(|id| (*id, Vec::new()))
            .collect::<BTreeMap<_, _>>();

        for (owner, component) in &self.components {
            if let Some(root) = component.condition {
                collect_edges(owner, root, false, &mut graph);
            }
        }

        for edges in graph.values_mut() {
            edges.sort();
            edges.dedup();
        }

        graph
    }

    fn evaluate_component(
        &self,
        component_id: &'static str,
        snapshot: &ConditionFactSnapshot,
        states: &mut BTreeMap<&'static str, bool>,
        decisions: &mut Vec<ConditionDecision>,
    ) -> Result<bool, ConditionError> {
        if let Some(value) = states.get(component_id) {
            return Ok(*value);
        }

        let component = self.components[component_id];
        let outcome = match component.condition {
            Some(root) => self.evaluate_node(component_id, root, snapshot, states, decisions)?,
            None => true,
        };

        states.insert(component_id, outcome);

        Ok(outcome)
    }

    fn evaluate_node(
        &self,
        owner: &'static str,
        node: &'static ConditionDescriptor,
        snapshot: &ConditionFactSnapshot,
        states: &mut BTreeMap<&'static str, bool>,
        decisions: &mut Vec<ConditionDecision>,
    ) -> Result<bool, ConditionError> {
        let outcome = match node.predicate {
            ConditionPredicate::ConfigBool(fact) => snapshot.facts[&fact]
                .as_bool()
                .expect("validated boolean fact"),
            ConditionPredicate::ConfigEquals { fact, expected } => {
                expected.matches(&snapshot.facts[&fact])
            }
            ConditionPredicate::ComponentEligible(component) => {
                self.evaluate_component(component, snapshot, states, decisions)?
            }
            ConditionPredicate::ProviderMappingEligible(provider) => {
                self.evaluate_component(provider.component, snapshot, states, decisions)?
            }
            ConditionPredicate::All(children) => {
                let mut outcomes = Vec::with_capacity(children.len());
                let mut children = children.iter().collect::<Vec<_>>();
                children.sort_by_key(|child| child.id);

                for child in children {
                    outcomes.push(self.evaluate_node(owner, child, snapshot, states, decisions)?);
                }

                outcomes.into_iter().all(|outcome| outcome)
            }
            ConditionPredicate::Any(children) => {
                let mut outcomes = Vec::with_capacity(children.len());
                let mut children = children.iter().collect::<Vec<_>>();
                children.sort_by_key(|child| child.id);

                for child in children {
                    outcomes.push(self.evaluate_node(owner, child, snapshot, states, decisions)?);
                }

                outcomes.into_iter().any(|outcome| outcome)
            }
            ConditionPredicate::Not(child) => {
                !self.evaluate_node(owner, child, snapshot, states, decisions)?
            }
        };

        decisions.push(ConditionDecision {
            component_id: owner,
            condition_id: node.id,
            predicate: node.predicate.kind(),
            source: node.source,
            outcome,
        });

        Ok(outcome)
    }
}

fn validate_manual_overrides(components: &[ComponentDescriptor]) -> Result<(), ConditionError> {
    let mut by_type = HashMap::<_, Vec<_>>::new();

    for component in components {
        by_type
            .entry(component.ty.type_id)
            .or_default()
            .push(component);
    }

    for candidates in by_type.values() {
        let conditional = candidates
            .iter()
            .find(|component| component.condition.is_some());
        let manual = candidates.iter().find(|component| {
            component
                .effective_factory()
                .map(|factory| factory.is_none())
                .unwrap_or(false)
        });

        if candidates.len() > 1
            && let (Some(conditional), Some(_)) = (conditional, manual)
        {
            return Err(ConditionError::ConditionalManualOverride(conditional.id));
        }
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AvailabilityEdge {
    pub from: &'static str,
    pub to: &'static str,
    pub condition_id: &'static str,
    pub predicate: ConditionPredicateKind,
    pub negated: bool,
}

fn collect_edges(
    owner: &'static str,
    node: &'static ConditionDescriptor,
    negated: bool,
    graph: &mut BTreeMap<&'static str, Vec<AvailabilityEdge>>,
) {
    let target = match node.predicate {
        ConditionPredicate::ComponentEligible(component) => Some(component),
        ConditionPredicate::ProviderMappingEligible(provider) => Some(provider.component),
        _ => None,
    };

    if let Some(target) = target {
        graph
            .get_mut(owner)
            .expect("validated owner")
            .push(AvailabilityEdge {
                from: owner,
                to: target,
                condition_id: node.id,
                predicate: node.predicate.kind(),
                negated,
            });
    }

    match node.predicate {
        ConditionPredicate::All(children) | ConditionPredicate::Any(children) => {
            for child in children {
                collect_edges(owner, child, negated, graph);
            }
        }
        ConditionPredicate::Not(child) => collect_edges(owner, child, !negated, graph),
        _ => {}
    }
}

fn collect_dependencies(
    node: &'static ConditionDescriptor,
    dependencies: &mut BTreeSet<ConditionDependency>,
) {
    match node.predicate {
        ConditionPredicate::ConfigBool(fact) | ConditionPredicate::ConfigEquals { fact, .. } => {
            dependencies.insert(ConditionDependency::Config(fact));
        }
        ConditionPredicate::ComponentEligible(component) => {
            dependencies.insert(ConditionDependency::Component(component));
        }
        ConditionPredicate::ProviderMappingEligible(provider) => {
            dependencies.insert(ConditionDependency::Provider(provider));
        }
        ConditionPredicate::All(children) | ConditionPredicate::Any(children) => {
            for child in children {
                collect_dependencies(child, dependencies);
            }
        }
        ConditionPredicate::Not(child) => collect_dependencies(child, dependencies),
    }
}

fn find_cycle(
    component: &'static str,
    graph: &BTreeMap<&'static str, Vec<AvailabilityEdge>>,
    visiting: &mut Vec<AvailabilityEdge>,
    active: &mut BTreeSet<&'static str>,
    completed: &mut BTreeSet<&'static str>,
) -> Option<Vec<AvailabilityEdge>> {
    if completed.contains(component) {
        return None;
    }

    active.insert(component);

    for edge in &graph[component] {
        if active.contains(edge.to) {
            let position = visiting
                .iter()
                .position(|candidate| candidate.from == edge.to)
                .unwrap_or(visiting.len());
            let mut cycle = visiting[position..].to_vec();
            cycle.push(*edge);

            return Some(cycle);
        }

        visiting.push(*edge);

        if let Some(cycle) = find_cycle(edge.to, graph, visiting, active, completed) {
            return Some(cycle);
        }

        visiting.pop();
    }

    active.remove(component);
    completed.insert(component);

    None
}

#[derive(Debug, thiserror::Error)]
pub enum ConditionError {
    #[error(transparent)]
    Registry(crate::Error),
    #[error("duplicate component id: {0}")]
    DuplicateComponentId(&'static str),
    #[error("unknown component id: {0}")]
    UnknownComponentId(String),
    #[error("duplicate provider mapping id: {0}")]
    DuplicateProviderMapping(ProviderMappingId),
    #[error(
        "provider mapping for trait '{trait_type}' and qualifier '{qualifier}' has no component"
    )]
    MissingProviderComponent {
        trait_type: &'static str,
        qualifier: &'static str,
    },
    #[error("duplicate config fact descriptor: {0:?}")]
    DuplicateFactDescriptor(ConfigFactId),
    #[error("duplicate supplied config fact: {0:?}")]
    DuplicateFactValue(ConfigFactId),
    #[error("missing supplied config fact: {0:?}")]
    MissingFactValue(ConfigFactId),
    #[error("unknown supplied config fact: {0:?}")]
    UnknownFactValue(ConfigFactId),
    #[error("config fact kind mismatch for {fact:?}: expected {expected:?}, found {actual:?}")]
    FactKindMismatch {
        fact: ConfigFactId,
        expected: ConditionScalarKind,
        actual: ConditionScalarKind,
    },
    #[error("duplicate condition id '{condition_id}' on component '{component_id}'")]
    DuplicateConditionId {
        component_id: &'static str,
        condition_id: &'static str,
    },
    #[error(
        "condition '{condition_id}' on '{component_id}' references missing config fact {fact:?}"
    )]
    MissingFactReference {
        component_id: &'static str,
        condition_id: &'static str,
        fact: ConfigFactId,
    },
    #[error(
        "condition '{condition_id}' on '{component_id}' references missing component '{referenced}'"
    )]
    MissingComponentReference {
        component_id: &'static str,
        condition_id: &'static str,
        referenced: &'static str,
    },
    #[error(
        "condition '{condition_id}' on '{component_id}' references missing provider mapping {provider}"
    )]
    MissingProviderReference {
        component_id: &'static str,
        condition_id: &'static str,
        provider: ProviderMappingId,
    },
    #[error("conditional component '{component_id}' uses unsupported scope '{scope}'")]
    UnsupportedScope {
        component_id: &'static str,
        scope: upwell_core::ScopeId,
    },
    #[error("factory-less component '{0}' cannot be conditional")]
    ConditionalManualComponent(&'static str),
    #[error("manual registration cannot override conditional component '{0}'")]
    ConditionalManualOverride(&'static str),
    #[error("condition availability cycle: {0:?}")]
    AvailabilityCycle(Vec<AvailabilityEdge>),
}

#[cfg(test)]
mod tests;
