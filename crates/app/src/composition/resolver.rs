use super::diagnostic::{CompositionDiagnostic, CompositionDiagnostics};
use super::graph;
use super::selection::{self, Directives};
use super::{
    CompositionDirective, CompositionPhase, InstallationProvenance, PluginDeclaration, PluginId,
    PluginSlotId, SlotPolicy,
};

/// One effective plugin installation in deterministic construction order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedPlugin {
    id: PluginId,
    provenance: InstallationProvenance,
    phase: CompositionPhase,
    slot: Option<(PluginSlotId, SlotPolicy)>,
    relations: Box<[super::PluginRelation]>,
}

impl ResolvedPlugin {
    pub(super) fn from_declaration(
        declaration: &PluginDeclaration,
        slot: Option<(PluginSlotId, SlotPolicy)>,
    ) -> Self {
        Self {
            id: declaration.id(),
            provenance: declaration.provenance(),
            phase: declaration.provenance().phase(),
            slot,
            relations: declaration.relations().into(),
        }
    }

    /// Returns the exact plugin implementation identity.
    pub const fn id(&self) -> PluginId {
        self.id
    }

    /// Returns where and when this implementation was declared.
    pub const fn provenance(&self) -> InstallationProvenance {
        self.provenance
    }

    /// Returns whether this plugin is parser-visible or a dynamic extension.
    pub const fn phase(&self) -> CompositionPhase {
        self.phase
    }

    /// Returns the effective capability slot and its mutation policy.
    pub const fn slot(&self) -> Option<(PluginSlotId, SlotPolicy)> {
        self.slot
    }

    /// Returns the declaration's structural graph relations.
    pub fn relations(&self) -> &[super::PluginRelation] {
        &self.relations
    }
}

/// The selected implementation that explicitly replaced a slot default.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ReplacementDecision {
    slot: PluginSlotId,
    replaced: PluginId,
    replaced_provenance: InstallationProvenance,
    replacement: PluginId,
    provenance: InstallationProvenance,
}

impl ReplacementDecision {
    pub(super) fn new(
        slot: PluginSlotId,
        replaced: &PluginDeclaration,
        replacement: &PluginDeclaration,
    ) -> Self {
        Self {
            slot,
            replaced: replaced.id(),
            replaced_provenance: replaced.provenance(),
            replacement: replacement.id(),
            provenance: replacement.provenance(),
        }
    }

    /// Returns the capability slot whose provider changed.
    pub const fn slot(self) -> PluginSlotId {
        self.slot
    }

    /// Returns the displaced implementation.
    pub const fn replaced(self) -> PluginId {
        self.replaced
    }

    /// Returns where the displaced implementation was installed.
    pub const fn replaced_provenance(self) -> InstallationProvenance {
        self.replaced_provenance
    }

    /// Returns the selected replacement implementation.
    pub const fn replacement(self) -> PluginId {
        self.replacement
    }

    /// Returns the replacement declaration provenance.
    pub const fn provenance(self) -> InstallationProvenance {
        self.provenance
    }
}

/// An optional capability slot explicitly removed from the effective plan.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SuppressionDecision {
    slot: PluginSlotId,
    suppressed: PluginId,
    suppressed_provenance: InstallationProvenance,
    provenance: InstallationProvenance,
}

impl SuppressionDecision {
    pub(super) fn new(
        slot: PluginSlotId,
        suppressed: &PluginDeclaration,
        provenance: InstallationProvenance,
    ) -> Self {
        Self {
            slot,
            suppressed: suppressed.id(),
            suppressed_provenance: suppressed.provenance(),
            provenance,
        }
    }

    /// Returns the disabled capability slot.
    pub const fn slot(self) -> PluginSlotId {
        self.slot
    }

    /// Returns the implementation removed with the slot.
    pub const fn suppressed(self) -> PluginId {
        self.suppressed
    }

    /// Returns where the disabled implementation was installed.
    pub const fn suppressed_provenance(self) -> InstallationProvenance {
        self.suppressed_provenance
    }

    /// Returns the suppression declaration provenance.
    pub const fn provenance(self) -> InstallationProvenance {
        self.provenance
    }
}

/// The immutable parser-visible plugin plan resolved before argument parsing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EarlyPluginPlan {
    plan: PluginResolutionPlan,
}

impl EarlyPluginPlan {
    /// Returns the selected protocol definition identity.
    pub const fn protocol(&self) -> super::ProtocolId {
        self.plan.protocol
    }

    /// Returns effective early plugins in deterministic order.
    pub fn plugins(&self) -> &[ResolvedPlugin] {
        &self.plan.plugins
    }

    /// Finds an effective plugin by exact implementation identity.
    pub fn plugin(&self, id: PluginId) -> Option<&ResolvedPlugin> {
        self.plan.plugin(id)
    }

    /// Returns the implementation selected for an effective capability slot.
    pub fn slot(&self, slot: PluginSlotId) -> Option<&ResolvedPlugin> {
        self.plan.slot(slot)
    }

    /// Returns explicit replacement decisions in stable slot order.
    pub fn replacements(&self) -> &[ReplacementDecision] {
        &self.plan.replacements
    }

    /// Returns explicit optional-slot suppressions in stable slot order.
    pub fn suppressions(&self) -> &[SuppressionDecision] {
        &self.plan.suppressions
    }
}

/// The immutable effective plugin plan after monotonic late extension.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginResolutionPlan {
    protocol: super::ProtocolId,
    plugins: Box<[ResolvedPlugin]>,
    replacements: Box<[ReplacementDecision]>,
    suppressions: Box<[SuppressionDecision]>,
}

impl PluginResolutionPlan {
    /// Returns the selected protocol definition identity.
    pub const fn protocol(&self) -> super::ProtocolId {
        self.protocol
    }

    /// Returns all effective plugins, with the unchanged early order before late plugins.
    pub fn plugins(&self) -> &[ResolvedPlugin] {
        &self.plugins
    }

    /// Finds an effective plugin by exact implementation identity.
    pub fn plugin(&self, id: PluginId) -> Option<&ResolvedPlugin> {
        self.plugins.iter().find(|plugin| plugin.id == id)
    }

    /// Returns the implementation selected for an effective capability slot.
    pub fn slot(&self, slot: PluginSlotId) -> Option<&ResolvedPlugin> {
        self.plugins
            .iter()
            .find(|plugin| plugin.slot.is_some_and(|(id, _)| id == slot))
    }

    /// Returns explicit replacement decisions in stable slot order.
    pub fn replacements(&self) -> &[ReplacementDecision] {
        &self.replacements
    }

    /// Returns explicit optional-slot suppressions in stable slot order.
    pub fn suppressions(&self) -> &[SuppressionDecision] {
        &self.suppressions
    }
}

/// Resolves the immutable parser-visible plugin plan.
pub fn resolve_early_plugins(
    protocol: super::ProtocolId,
    directives: impl IntoIterator<Item = CompositionDirective>,
) -> Result<EarlyPluginPlan, CompositionDiagnostics> {
    let directives: Vec<_> = directives.into_iter().collect();
    let plan = resolve_phase(
        protocol,
        CompositionPhase::Early,
        &[],
        &directives,
        &[],
        &[],
    )?;

    Ok(EarlyPluginPlan { plan })
}

/// Extends an early plan with dynamic plugins without changing its identity or order.
pub fn extend_late_plugins(
    early: &EarlyPluginPlan,
    directives: impl IntoIterator<Item = CompositionDirective>,
) -> Result<PluginResolutionPlan, CompositionDiagnostics> {
    let directives: Vec<_> = directives.into_iter().collect();

    resolve_phase(
        early.plan.protocol,
        CompositionPhase::Late,
        &early.plan.plugins,
        &directives,
        &early.plan.replacements,
        &early.plan.suppressions,
    )
}

fn resolve_phase(
    protocol: super::ProtocolId,
    phase: CompositionPhase,
    prior: &[ResolvedPlugin],
    directives: &[CompositionDirective],
    prior_replacements: &[ReplacementDecision],
    prior_suppressions: &[SuppressionDecision],
) -> Result<PluginResolutionPlan, CompositionDiagnostics> {
    let mut diagnostics = Vec::new();
    let directives = Directives::collect(phase, directives, &mut diagnostics);
    let selection = selection::select(
        phase,
        &directives,
        prior,
        prior_suppressions,
        &mut diagnostics,
    );

    let mut effective = selection.plugins;
    let graph = graph::validate(phase, &effective, prior, &mut diagnostics);
    if !diagnostics
        .iter()
        .any(CompositionDiagnostic::makes_graph_ambiguous)
    {
        diagnostics.extend(graph::cycles(phase, &effective, &graph));
    }

    if !diagnostics.is_empty() {
        return Err(CompositionDiagnostics::new(diagnostics));
    }

    effective = graph::order(effective, &graph);

    let mut plugins = prior.to_vec();
    let mut all_replacements = prior_replacements.to_vec();
    let mut all_suppressions = prior_suppressions.to_vec();

    plugins.extend(effective);
    all_replacements.extend(selection.replacements);
    all_suppressions.extend(selection.suppressions);
    all_replacements.sort();
    all_suppressions.sort();

    Ok(PluginResolutionPlan {
        protocol,
        plugins: plugins.into_boxed_slice(),
        replacements: all_replacements.into_boxed_slice(),
        suppressions: all_suppressions.into_boxed_slice(),
    })
}
