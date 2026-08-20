use std::any::TypeId;
use std::collections::{BTreeMap, HashMap};
use std::fmt;

use upwell_core::{ConditionScalar, ConfigFactDescriptor, ConfigFactId, ProviderMappingId};

use crate::{ComponentDescriptor, ComponentRegistry, ProviderDescriptor, ProviderSelectionModel};

mod error;
mod evaluation;
mod graph;
mod validation;

pub use error::ConditionError;
pub use graph::{AvailabilityEdge, ConditionDependency};

/// Typed config facts supplied to one catalog evaluation.
#[derive(Clone, Default)]
pub struct ConditionFactSnapshot {
    pub(super) facts: BTreeMap<ConfigFactId, ConditionScalar>,
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
    pub predicate: upwell_core::ConditionPredicateKind,
    pub callback_kind: Option<&'static str>,
    pub source: upwell_core::DescriptorSource,
    pub outcome: bool,
}

/// Deterministic eligibility result for one supplied fact snapshot.
#[derive(Clone, Debug)]
pub struct ConditionEvaluation {
    pub(super) eligible: ComponentRegistry,
    pub(super) components: BTreeMap<&'static str, bool>,
    pub(super) providers: BTreeMap<ProviderMappingId, bool>,
    pub(super) decisions: Vec<ConditionDecision>,
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

/// Eligibility decisions whose ordinary DI graph has also passed validation.
pub struct ValidatedConditionEvaluation {
    pub(super) evaluation: ConditionEvaluation,
    pub(super) selection: ProviderSelectionModel,
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

/// Validated static inputs for deterministic condition evaluation.
pub struct ConditionCatalog {
    pub(super) components: BTreeMap<&'static str, ComponentDescriptor>,
    pub(super) component_order: Vec<&'static str>,
    pub(super) providers: Vec<(ProviderMappingId, ProviderDescriptor)>,
    pub(super) facts: BTreeMap<ConfigFactId, ConfigFactDescriptor>,
    pub(super) provider_order: HashMap<TypeId, HashMap<TypeId, usize>>,
}

#[cfg(test)]
mod tests;
