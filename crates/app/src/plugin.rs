//! Retained plugin installation, contribution collection, and deterministic lowering.

mod catalog;
mod contribution;

use crate::{PluginId, PluginRelation};

#[cfg(feature = "cli")]
use crate::PluginCliRegistrar;

pub(crate) use catalog::PluginCatalog;
pub use catalog::{ApplicationPluginRegistrar, EarlyPluginCatalog, ProtocolPluginRegistrar};
pub use contribution::{
    EffectivePluginPlan, PluginContribution, PluginContributionKind, PluginContributions,
    PluginPlanError,
};

/// A typed application extension that emits app-neutral contributions before validation.
pub trait Plugin: Send + 'static {
    /// Stable implementation identity used for composition and diagnostics.
    const ID: PluginId;

    /// Static dependency, conflict, and ordering relations.
    const RELATIONS: &'static [PluginRelation] = &[];

    /// Emits deterministic app-neutral contributions for the effective application plan.
    fn contribute(self, contributions: &mut PluginContributions);

    /// Folds link-time-discovered plugin state into this retained instance.
    fn auto_discover(&mut self) {}

    /// Declares optional parser-visible CLI facets without consuming plugin state.
    #[cfg(feature = "cli")]
    fn cli(&self, _cli: &mut PluginCliRegistrar) {}
}

/// A plugin that can be synchronously constructed from explicit options.
pub trait PluginWithOptions: Plugin {
    /// User-facing construction options retained by the resulting plugin as needed.
    type Options;

    /// Constructs the plugin from explicit options before deterministic composition.
    fn from_options(options: Self::Options) -> Self;
}

#[cfg(test)]
mod tests;
