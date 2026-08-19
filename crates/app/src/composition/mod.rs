//! Stable identities and deterministic planning for application plugin composition.

mod diagnostic;
mod graph;
mod id;
mod model;
mod resolver;
mod selection;

pub use diagnostic::{
    CompositionDiagnostic, CompositionDiagnostics, CompositionEdge, CompositionTarget,
};
pub use id::{
    ContributionId, IdErrorKind, InvalidCompositionId, PluginId, PluginSlotId, ProtocolId,
};
pub use model::{
    CompositionDirective, CompositionPhase, ContributionProvenance, Contributor,
    InstallationOrigin, InstallationProvenance, PluginDeclaration, PluginRelation, RelationKind,
    RelationTarget, SlotPolicy,
};
pub use resolver::{
    EarlyPluginPlan, PluginResolutionPlan, ReplacementDecision, ResolvedPlugin,
    SuppressionDecision, extend_late_plugins, resolve_early_plugins,
};

#[cfg(test)]
mod tests;
