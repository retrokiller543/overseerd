use std::fmt;

use thiserror::Error;

use super::{
    CompositionPhase, InstallationProvenance, PluginId, PluginSlotId, RelationKind, RelationTarget,
    SlotPolicy,
};

/// A resolved graph endpoint used by cycle diagnostics.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CompositionTarget {
    plugin: PluginId,
    provenance: InstallationProvenance,
}

impl CompositionTarget {
    pub(crate) const fn new(plugin: PluginId, provenance: InstallationProvenance) -> Self {
        Self { plugin, provenance }
    }

    /// Returns the exact plugin implementation at this endpoint.
    pub const fn plugin(self) -> PluginId {
        self.plugin
    }

    /// Returns where the endpoint plugin was installed.
    pub const fn provenance(self) -> InstallationProvenance {
        self.provenance
    }
}

/// One directed graph edge in a canonical composition cycle.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CompositionEdge {
    from: CompositionTarget,
    to: CompositionTarget,
    kinds: Box<[RelationKind]>,
}

impl CompositionEdge {
    pub(crate) fn new(
        from: CompositionTarget,
        to: CompositionTarget,
        kinds: Box<[RelationKind]>,
    ) -> Self {
        Self { from, to, kinds }
    }

    /// Returns the edge source plugin.
    pub const fn from(&self) -> CompositionTarget {
        self.from
    }

    /// Returns the edge destination plugin.
    pub const fn to(&self) -> CompositionTarget {
        self.to
    }

    /// Returns every relation kind that produced this edge.
    pub fn kinds(&self) -> &[RelationKind] {
        &self.kinds
    }
}

/// A typed structural failure found while resolving plugin composition.
#[derive(Clone, Debug, Eq, Error, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum CompositionDiagnostic {
    /// A non-framework declaration claims the reserved framework namespace.
    #[error("plugin '{plugin}' uses the reserved 'overseerd/' namespace ({provenance:?})")]
    ReservedNamespace {
        plugin: PluginId,
        provenance: InstallationProvenance,
    },
    /// The same plugin implementation was declared more than once.
    #[error("plugin '{id}' is declared more than once ({first:?} and {second:?})")]
    DuplicatePlugin {
        id: PluginId,
        first: InstallationProvenance,
        second: InstallationProvenance,
    },
    /// More than one ordinary installation provides the same capability slot.
    #[error(
        "slot '{slot}' has multiple providers: '{first_plugin}' ({first:?}) and '{second_plugin}' ({second:?})"
    )]
    DuplicateSlot {
        slot: PluginSlotId,
        first_plugin: PluginId,
        first: InstallationProvenance,
        second_plugin: PluginId,
        second: InstallationProvenance,
    },
    /// A required exact plugin or capability slot is absent.
    #[error("plugin '{plugin}' requires missing target {target:?}")]
    MissingDependency {
        plugin: PluginId,
        provenance: InstallationProvenance,
        target: RelationTarget,
    },
    /// A dependency resolves back to the source plugin.
    #[error("plugin '{plugin}' depends on itself through {target:?}")]
    SelfDependency {
        plugin: PluginId,
        provenance: InstallationProvenance,
        target: RelationTarget,
    },
    /// A conflict resolves back to the source plugin.
    #[error("plugin '{plugin}' conflicts with itself through {target:?}")]
    SelfConflict {
        plugin: PluginId,
        provenance: InstallationProvenance,
        target: RelationTarget,
    },
    /// Two effective plugins conflict with each other.
    #[error("plugins '{left}' ({left_provenance:?}) and '{right}' ({right_provenance:?}) conflict")]
    Conflict {
        left: PluginId,
        left_provenance: InstallationProvenance,
        right: PluginId,
        right_provenance: InstallationProvenance,
    },
    /// An ordering relation resolves back to the source plugin.
    #[error("plugin '{plugin}' orders itself {kind:?} through {target:?}")]
    SelfOrdering {
        plugin: PluginId,
        provenance: InstallationProvenance,
        kind: RelationKind,
        target: RelationTarget,
    },
    /// A replacement refers to a slot that is not declared.
    #[error("replacement refers to missing slot '{slot}' ({provenance:?})")]
    ReplacementTargetMissing {
        slot: PluginSlotId,
        provenance: InstallationProvenance,
    },
    /// A slot policy does not permit replacement.
    #[error("slot '{slot}' with policy {policy:?} cannot be replaced ({provenance:?})")]
    ReplacementForbidden {
        slot: PluginSlotId,
        policy: SlotPolicy,
        provenance: InstallationProvenance,
    },
    /// More than one directive attempts to replace the same slot.
    #[error("slot '{slot}' has multiple replacements ({first:?} and {second:?})")]
    MultipleReplacements {
        slot: PluginSlotId,
        first: InstallationProvenance,
        second: InstallationProvenance,
    },
    /// More than one directive attempts to suppress the same slot.
    #[error("slot '{slot}' has multiple suppressions ({first:?} and {second:?})")]
    MultipleSuppressions {
        slot: PluginSlotId,
        first: InstallationProvenance,
        second: InstallationProvenance,
    },
    /// A replacement declaration attempts to provide an unrelated slot.
    #[error(
        "replacement plugin '{plugin}' for slot '{target}' also declares slot '{declared}' ({provenance:?})"
    )]
    ReplacementDeclaresSlot {
        target: PluginSlotId,
        declared: PluginSlotId,
        plugin: PluginId,
        provenance: InstallationProvenance,
    },
    /// A suppression refers to a slot that is not declared.
    #[error("suppression refers to missing slot '{slot}' ({provenance:?})")]
    SuppressionTargetMissing {
        slot: PluginSlotId,
        provenance: InstallationProvenance,
    },
    /// A slot policy does not permit suppression.
    #[error("slot '{slot}' with policy {policy:?} cannot be suppressed ({provenance:?})")]
    SuppressionForbidden {
        slot: PluginSlotId,
        policy: SlotPolicy,
        provenance: InstallationProvenance,
    },
    /// Replacement and suppression directives compete for the same slot.
    #[error("slot '{slot}' has incompatible directives ({first:?} and {second:?})")]
    ConflictingSlotDirectives {
        slot: PluginSlotId,
        first: InstallationProvenance,
        second: InstallationProvenance,
    },
    /// A directive was supplied to the wrong resolution phase.
    #[error(
        "plugin directive from {provenance:?} belongs to {actual:?} composition, not {expected:?}"
    )]
    UnexpectedPhase {
        expected: CompositionPhase,
        actual: CompositionPhase,
        provenance: InstallationProvenance,
    },
    /// A late declaration attempts to mutate a parser-visible early slot.
    #[error("late composition cannot mutate early slot '{slot}' ({provenance:?})")]
    LateSlotMutation {
        slot: PluginSlotId,
        provenance: InstallationProvenance,
    },
    /// A late plugin attempts to move before an early plugin.
    #[error(
        "late plugin '{plugin}' cannot be ordered before early plugin '{target}' ({provenance:?})"
    )]
    LateOrderingBeforeEarly {
        plugin: PluginId,
        target: PluginId,
        provenance: InstallationProvenance,
    },
    /// An early declaration would need to move after a newly installed late plugin.
    #[error(
        "early plugin '{plugin}' cannot be ordered after late plugin '{target}' ({provenance:?})"
    )]
    EarlyOrderingAfterLate {
        plugin: PluginId,
        target: PluginId,
        provenance: InstallationProvenance,
    },
    /// A dependency or ordering cycle prevents deterministic resolution.
    #[error("{phase:?} plugin composition contains a cycle: {display}")]
    Cycle {
        phase: CompositionPhase,
        members: Box<[CompositionTarget]>,
        steps: Box<[CompositionEdge]>,
        display: String,
    },
}

impl CompositionDiagnostic {
    pub(crate) const fn makes_graph_ambiguous(&self) -> bool {
        matches!(
            self,
            Self::DuplicatePlugin { .. }
                | Self::DuplicateSlot { .. }
                | Self::MultipleReplacements { .. }
                | Self::MultipleSuppressions { .. }
                | Self::ConflictingSlotDirectives { .. }
                | Self::LateSlotMutation { .. }
        )
    }
}

/// A deterministic non-empty report of independent composition failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionDiagnostics {
    diagnostics: Box<[CompositionDiagnostic]>,
}

impl CompositionDiagnostics {
    pub(crate) fn new(mut diagnostics: Vec<CompositionDiagnostic>) -> Self {
        diagnostics.sort();
        diagnostics.dedup();
        assert!(
            !diagnostics.is_empty(),
            "composition diagnostics are non-empty"
        );

        Self {
            diagnostics: diagnostics.into_boxed_slice(),
        }
    }

    /// Returns every diagnostic in canonical order.
    pub fn as_slice(&self) -> &[CompositionDiagnostic] {
        &self.diagnostics
    }

    /// Returns the number of independent failures in this report.
    pub fn len(&self) -> usize {
        self.diagnostics.len()
    }

    /// Returns whether this report contains no diagnostics.
    pub fn is_empty(&self) -> bool {
        false
    }
}

impl fmt::Display for CompositionDiagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }

            write!(f, "{diagnostic}")?;
        }

        Ok(())
    }
}

impl std::error::Error for CompositionDiagnostics {}
