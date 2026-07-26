//! Retained plugin installation, contribution collection, and deterministic lowering.

mod catalog;
mod contribution;

use crate::{PluginId, PluginRelation};

pub(crate) use catalog::PluginCatalog;
pub use catalog::{ApplicationPluginRegistrar, ProtocolPluginRegistrar};
pub use contribution::{
    EffectivePluginPlan, PluginContribution, PluginContributionKind, PluginContributions,
    PluginPlanError,
};

/// A typed application extension that emits app-neutral contributions before validation.
pub trait Plugin: Default + Send + 'static {
    /// Stable implementation identity used for composition and diagnostics.
    const ID: PluginId;

    /// Static dependency, conflict, and ordering relations.
    const RELATIONS: &'static [PluginRelation] = &[];

    /// Folds link-time-discovered plugin state into this retained instance.
    fn auto_discover(&mut self) {}

    /// Emits deterministic app-neutral contributions for the effective application plan.
    fn contribute(self, contributions: &mut PluginContributions);
}

#[cfg(test)]
mod tests;
