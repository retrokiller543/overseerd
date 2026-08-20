use std::collections::HashMap;
use std::sync::Arc;

use upwell_core::{RuntimeGenerationId, ScopeId};
use upwell_di::{ComponentDescriptor, ConditionEvaluation, EffectiveGraph, ProviderSelectionModel};

use crate::AppRegistry;
use crate::scope::{PreparedScopeTopology, ScopePlan, SeedDestination};

/// A validated candidate effective graph plus the plans used by future scope openings.
///
/// Preparing this value performs no construction and does not mutate active runtime state.
pub struct CandidateGraph {
    graph: EffectiveGraph,
    components: Box<[ComponentDescriptor]>,
    singleton_order: Box<[ComponentDescriptor]>,
    scope_orders: HashMap<ScopeId, Box<[ComponentDescriptor]>>,
    seed_destinations: HashMap<std::any::TypeId, SeedDestination>,
}

impl CandidateGraph {
    /// Evaluates application-specific graph invariants and freezes future scope plans.
    pub fn prepare(
        base_generation: RuntimeGenerationId,
        registry: &AppRegistry,
        topology: &PreparedScopeTopology,
    ) -> crate::Result<Self> {
        Self::prepare_registry(
            base_generation,
            registry,
            registry.component_registry(),
            topology,
        )
    }

    /// Prepares a candidate from a complete condition evaluation while retaining the
    /// application's config-binding contract.
    pub fn prepare_evaluation(
        base_generation: RuntimeGenerationId,
        registry: &AppRegistry,
        evaluation: &ConditionEvaluation,
        topology: &PreparedScopeTopology,
    ) -> crate::Result<Self> {
        Self::prepare_registry(
            base_generation,
            registry,
            evaluation.eligible_registry().clone(),
            topology,
        )
    }

    fn prepare_registry(
        base_generation: RuntimeGenerationId,
        registry: &AppRegistry,
        component_registry: upwell_di::ComponentRegistry,
        topology: &PreparedScopeTopology,
    ) -> crate::Result<Self> {
        let components = component_registry.resolved_components()?;
        let selection = Arc::new(component_registry.provider_selection_model(&components)?);

        component_registry.validate_with_scope_reachability_using(
            &components,
            &selection,
            |consumer, dependency| topology.is_reachable(&consumer, &dependency),
        )?;
        registry.validate_configs(&components)?;

        let graph = EffectiveGraph::from_validated(
            base_generation,
            &components,
            Arc::clone(&selection),
            |consumer, dependency| topology.is_reachable(&consumer, &dependency),
        )?;
        let scopes = ScopePlan::partition(&components, &selection, topology)?;
        let singleton_order = graph
            .construction_order()
            .iter()
            .filter_map(|id| components.iter().find(|component| component.id == *id))
            .copied()
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let scope_orders = scopes
            .orders
            .into_iter()
            .map(|(scope, order)| (scope, order.into_boxed_slice()))
            .collect();

        Ok(Self {
            graph,
            components: components.into_boxed_slice(),
            singleton_order,
            scope_orders,
            seed_destinations: scopes.seed_destinations,
        })
    }

    pub fn graph(&self) -> &EffectiveGraph {
        &self.graph
    }

    pub fn components(&self) -> &[ComponentDescriptor] {
        &self.components
    }

    pub fn provider_selection(&self) -> &Arc<ProviderSelectionModel> {
        self.graph.provider_selection()
    }

    pub fn singleton_order(&self) -> &[ComponentDescriptor] {
        &self.singleton_order
    }

    pub fn scope_order(&self, scope: &ScopeId) -> Option<&[ComponentDescriptor]> {
        self.scope_orders.get(scope).map(Box::as_ref)
    }

    pub fn seed_destination(&self, type_id: std::any::TypeId) -> Option<(ScopeId, &'static str)> {
        self.seed_destinations
            .get(&type_id)
            .map(|destination| (destination.scope, destination.type_name))
    }
}
