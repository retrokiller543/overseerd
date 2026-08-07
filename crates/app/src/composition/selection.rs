use std::collections::BTreeMap;

use upwell_core::NamespacedIdType;

use super::diagnostic::CompositionDiagnostic;
use super::model::DirectiveKind;
use super::resolver::{ReplacementDecision, ResolvedPlugin, SuppressionDecision};
use super::{
    CompositionPhase, InstallationProvenance, PluginDeclaration, PluginId, PluginSlotId, SlotPolicy,
};

/// Effective plugin selection and the directives that produced it.
pub(super) struct Selection {
    pub(super) plugins: Vec<ResolvedPlugin>,
    pub(super) replacements: Vec<ReplacementDecision>,
    pub(super) suppressions: Vec<SuppressionDecision>,
    pub(super) graph_ambiguous: bool,
}

/// Canonical structural directives for one resolution phase.
pub(super) struct Directives<'a> {
    pub(super) declarations: Vec<&'a PluginDeclaration>,
    pub(super) replacements: BTreeMap<PluginSlotId, Vec<&'a PluginDeclaration>>,
    pub(super) suppressions: BTreeMap<PluginSlotId, Vec<InstallationProvenance>>,
}

impl<'a> Directives<'a> {
    /// Separates and canonicalizes declarations for the expected phase.
    pub(super) fn collect(
        phase: CompositionPhase,
        directives: &'a [super::CompositionDirective],
        diagnostics: &mut Vec<CompositionDiagnostic>,
    ) -> Self {
        let mut declarations = Vec::new();
        let mut replacements: BTreeMap<PluginSlotId, Vec<&PluginDeclaration>> = BTreeMap::new();
        let mut suppressions: BTreeMap<PluginSlotId, Vec<InstallationProvenance>> = BTreeMap::new();

        for directive in directives {
            let provenance = directive.provenance();
            let actual = provenance.phase();

            match directive.kind() {
                DirectiveKind::Replace { slot, .. } | DirectiveKind::Suppress { slot, .. }
                    if slot.is_in_namespace(upwell_core::FRAMEWORK_NAMESPACE) =>
                {
                    diagnostics.push(CompositionDiagnostic::ReservedSlotNamespace {
                        slot: *slot,
                        provenance,
                    });

                    continue;
                }
                DirectiveKind::Install(_)
                | DirectiveKind::Replace { .. }
                | DirectiveKind::Suppress { .. } => {}
            }

            if actual != phase {
                diagnostics.push(CompositionDiagnostic::UnexpectedPhase {
                    expected: phase,
                    actual,
                    provenance,
                });

                continue;
            }

            match directive.kind() {
                DirectiveKind::Install(plugin) => declarations.push(plugin),
                DirectiveKind::Replace { slot, plugin } => {
                    replacements.entry(*slot).or_default().push(plugin);
                }
                DirectiveKind::Suppress { slot, provenance } => {
                    suppressions.entry(*slot).or_default().push(*provenance);
                }
            }
        }

        declarations.sort_by_key(|plugin| (*plugin).clone());

        for plugins in replacements.values_mut() {
            plugins.sort_by_key(|plugin| (*plugin).clone());
        }

        for sites in suppressions.values_mut() {
            sites.sort();
        }

        Self {
            declarations,
            replacements,
            suppressions,
        }
    }
}

/// Validates declarations and applies slot replacement and suppression.
pub(super) fn select(
    phase: CompositionPhase,
    directives: &Directives<'_>,
    prior: &[ResolvedPlugin],
    prior_suppressions: &[SuppressionDecision],
    diagnostics: &mut Vec<CompositionDiagnostic>,
) -> Selection {
    let duplicate_plugins = validate_plugin_ids(
        prior,
        &directives.declarations,
        &directives.replacements,
        diagnostics,
    );

    let (base_slots, duplicate_slots) = validate_slots(&directives.declarations, diagnostics);

    select_plugins(
        phase,
        &directives.declarations,
        base_slots,
        &directives.replacements,
        &directives.suppressions,
        prior,
        prior_suppressions,
        duplicate_plugins || duplicate_slots,
        diagnostics,
    )
}

fn validate_plugin_ids<'a>(
    prior: &[ResolvedPlugin],
    declarations: &[&'a PluginDeclaration],
    replacements: &BTreeMap<PluginSlotId, Vec<&'a PluginDeclaration>>,
    diagnostics: &mut Vec<CompositionDiagnostic>,
) -> bool {
    let mut by_id: BTreeMap<PluginId, Vec<InstallationProvenance>> = BTreeMap::new();
    let mut duplicates = false;

    for plugin in prior {
        by_id
            .entry(plugin.id())
            .or_default()
            .push(plugin.provenance());
    }

    for declaration in declarations
        .iter()
        .copied()
        .chain(replacements.values().flatten().copied())
    {
        if declaration
            .id()
            .is_in_namespace(upwell_core::FRAMEWORK_NAMESPACE)
        {
            diagnostics.push(CompositionDiagnostic::ReservedNamespace {
                plugin: declaration.id(),
                provenance: declaration.provenance(),
            });
        }

        by_id
            .entry(declaration.id())
            .or_default()
            .push(declaration.provenance());
    }

    for (id, sites) in &mut by_id {
        sites.sort();

        duplicates |= sites.len() > 1;

        for pair in sites.windows(2) {
            diagnostics.push(CompositionDiagnostic::DuplicatePlugin {
                id: *id,
                first: pair[0],
                second: pair[1],
            });
        }
    }

    duplicates
}

fn validate_slots<'a>(
    declarations: &[&'a PluginDeclaration],
    diagnostics: &mut Vec<CompositionDiagnostic>,
) -> (BTreeMap<PluginSlotId, &'a PluginDeclaration>, bool) {
    let mut providers: BTreeMap<PluginSlotId, Vec<&PluginDeclaration>> = BTreeMap::new();
    let mut duplicates = false;

    for declaration in declarations.iter().copied() {
        let Some((slot, _)) = declaration.slot() else {
            continue;
        };

        if slot.is_in_namespace(upwell_core::FRAMEWORK_NAMESPACE) {
            diagnostics.push(CompositionDiagnostic::ReservedSlotNamespace {
                slot,
                provenance: declaration.provenance(),
            });
        }

        providers.entry(slot).or_default().push(declaration);
    }

    for (slot, declarations) in &mut providers {
        declarations.sort_by_key(|declaration| (declaration.id(), declaration.provenance()));
        duplicates |= declarations.len() > 1;

        for pair in declarations.windows(2) {
            let first = pair[0];
            let second = pair[1];

            diagnostics.push(CompositionDiagnostic::DuplicateSlot {
                slot: *slot,
                first_plugin: first.id(),
                first: first.provenance(),
                second_plugin: second.id(),
                second: second.provenance(),
            });
        }
    }

    let slots = providers
        .into_iter()
        .map(|(slot, declarations)| (slot, declarations[0]))
        .collect();

    (slots, duplicates)
}

#[allow(clippy::too_many_arguments)]
fn select_plugins(
    phase: CompositionPhase,
    declarations: &[&PluginDeclaration],
    base_slots: BTreeMap<PluginSlotId, &PluginDeclaration>,
    replacements: &BTreeMap<PluginSlotId, Vec<&PluginDeclaration>>,
    suppressions: &BTreeMap<PluginSlotId, Vec<InstallationProvenance>>,
    prior: &[ResolvedPlugin],
    prior_suppressions: &[SuppressionDecision],
    mut graph_ambiguous: bool,
    diagnostics: &mut Vec<CompositionDiagnostic>,
) -> Selection {
    let prior_slots: BTreeMap<_, _> = prior
        .iter()
        .filter_map(|plugin| plugin.slot().map(|(slot, policy)| (slot, (plugin, policy))))
        .collect();
    let prior_slot_ids: std::collections::BTreeSet<_> = prior_slots
        .keys()
        .copied()
        .chain(prior_suppressions.iter().map(|decision| decision.slot()))
        .collect();
    let mut selected: BTreeMap<PluginId, ResolvedPlugin> = declarations
        .iter()
        .map(|declaration| {
            (
                declaration.id(),
                ResolvedPlugin::from_declaration(declaration, declaration.slot()),
            )
        })
        .collect();
    let mut decisions = Vec::new();
    let mut disabled = Vec::new();

    if phase == CompositionPhase::Late {
        for (slot, declaration) in &base_slots {
            if prior_slot_ids.contains(slot) {
                graph_ambiguous = true;
                diagnostics.push(CompositionDiagnostic::LateSlotMutation {
                    slot: *slot,
                    provenance: declaration.provenance(),
                });
            }
        }
    }

    graph_ambiguous |= apply_replacements(
        phase,
        &base_slots,
        replacements,
        &prior_slot_ids,
        &mut selected,
        &mut decisions,
        diagnostics,
    );
    graph_ambiguous |= apply_suppressions(
        phase,
        &base_slots,
        replacements,
        suppressions,
        &prior_slot_ids,
        &mut selected,
        &mut disabled,
        diagnostics,
    );

    Selection {
        plugins: selected.into_values().collect(),
        replacements: decisions,
        suppressions: disabled,
        graph_ambiguous,
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_replacements(
    phase: CompositionPhase,
    base_slots: &BTreeMap<PluginSlotId, &PluginDeclaration>,
    replacements: &BTreeMap<PluginSlotId, Vec<&PluginDeclaration>>,
    prior_slots: &std::collections::BTreeSet<PluginSlotId>,
    selected: &mut BTreeMap<PluginId, ResolvedPlugin>,
    decisions: &mut Vec<ReplacementDecision>,
    diagnostics: &mut Vec<CompositionDiagnostic>,
) -> bool {
    let mut graph_ambiguous = false;

    for (slot, replacement_declarations) in replacements {
        if replacement_declarations.len() > 1 {
            graph_ambiguous = true;
            let sites: Vec<_> = replacement_declarations
                .iter()
                .map(|plugin| plugin.provenance())
                .collect();

            for pair in sites.windows(2) {
                diagnostics.push(CompositionDiagnostic::MultipleReplacements {
                    slot: *slot,
                    first: pair[0],
                    second: pair[1],
                });
            }

            continue;
        }

        let replacement = replacement_declarations[0];

        if phase == CompositionPhase::Late && prior_slots.contains(slot) {
            diagnostics.push(CompositionDiagnostic::LateSlotMutation {
                slot: *slot,
                provenance: replacement.provenance(),
            });

            continue;
        }

        let Some(target) = base_slots.get(slot).copied() else {
            diagnostics.push(CompositionDiagnostic::ReplacementTargetMissing {
                slot: *slot,
                provenance: replacement.provenance(),
            });

            continue;
        };
        let (_, policy) = target.slot().expect("slot index contains slotted plugin");

        if policy == SlotPolicy::Fixed {
            diagnostics.push(CompositionDiagnostic::ReplacementForbidden {
                slot: *slot,
                policy,
                provenance: replacement.provenance(),
            });

            continue;
        }

        if let Some((declared, _)) = replacement.slot()
            && declared != *slot
        {
            diagnostics.push(CompositionDiagnostic::ReplacementDeclaresSlot {
                target: *slot,
                declared,
                plugin: replacement.id(),
                provenance: replacement.provenance(),
            });

            continue;
        }

        selected.remove(&target.id());
        selected.insert(
            replacement.id(),
            ResolvedPlugin::from_declaration(replacement, Some((*slot, policy))),
        );
        decisions.push(ReplacementDecision::new(*slot, target, replacement));
    }

    graph_ambiguous
}

#[allow(clippy::too_many_arguments)]
fn apply_suppressions(
    phase: CompositionPhase,
    base_slots: &BTreeMap<PluginSlotId, &PluginDeclaration>,
    replacements: &BTreeMap<PluginSlotId, Vec<&PluginDeclaration>>,
    suppressions: &BTreeMap<PluginSlotId, Vec<InstallationProvenance>>,
    prior_slots: &std::collections::BTreeSet<PluginSlotId>,
    selected: &mut BTreeMap<PluginId, ResolvedPlugin>,
    disabled: &mut Vec<SuppressionDecision>,
    diagnostics: &mut Vec<CompositionDiagnostic>,
) -> bool {
    let mut graph_ambiguous = false;

    for (slot, sites) in suppressions {
        if sites.len() > 1 {
            for pair in sites.windows(2) {
                diagnostics.push(CompositionDiagnostic::MultipleSuppressions {
                    slot: *slot,
                    first: pair[0],
                    second: pair[1],
                });
            }
        }

        if phase == CompositionPhase::Late && prior_slots.contains(slot) {
            diagnostics.push(CompositionDiagnostic::LateSlotMutation {
                slot: *slot,
                provenance: sites[0],
            });

            continue;
        }

        if let Some(replacement) = replacements.get(slot).and_then(|items| items.first()) {
            graph_ambiguous = true;
            diagnostics.push(CompositionDiagnostic::ConflictingSlotDirectives {
                slot: *slot,
                first: replacement.provenance().min(sites[0]),
                second: replacement.provenance().max(sites[0]),
            });

            continue;
        }

        let Some(target) = base_slots.get(slot).copied() else {
            diagnostics.push(CompositionDiagnostic::SuppressionTargetMissing {
                slot: *slot,
                provenance: sites[0],
            });

            continue;
        };
        let (_, policy) = target.slot().expect("slot index contains slotted plugin");

        if policy != SlotPolicy::Optional {
            diagnostics.push(CompositionDiagnostic::SuppressionForbidden {
                slot: *slot,
                policy,
                provenance: sites[0],
            });

            continue;
        }

        selected.remove(&target.id());
        disabled.push(SuppressionDecision::new(*slot, target, sites[0]));
    }

    graph_ambiguous
}
