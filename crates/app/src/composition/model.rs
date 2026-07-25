use super::{ContributionId, PluginId, PluginSlotId, ProtocolId};

/// The stage at which a plugin installation becomes part of the application definition.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CompositionPhase {
    /// Parser-visible declarations resolved before process arguments are parsed.
    Early,
    /// Dynamic declarations added during application configuration.
    Late,
}

/// The semantic source of a plugin installation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum InstallationOrigin {
    /// A framework-owned mandatory or default installation.
    Framework,
    /// A default supplied by the selected protocol definition.
    ProtocolDefault(ProtocolId),
    /// A plugin declared statically by the application.
    ApplicationDeclaration,
    /// A plugin installed dynamically while configuring the application.
    ApplicationConfiguration,
}

impl InstallationOrigin {
    /// Returns the composition phase implied by this origin.
    pub const fn phase(self) -> CompositionPhase {
        match self {
            Self::Framework | Self::ProtocolDefault(_) | Self::ApplicationDeclaration => {
                CompositionPhase::Early
            }
            Self::ApplicationConfiguration => CompositionPhase::Late,
        }
    }
}

/// Provenance for one plugin declaration or slot directive.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InstallationProvenance {
    origin: InstallationOrigin,
    ordinal: u32,
}

impl InstallationProvenance {
    /// Creates installation provenance with an origin-local declaration ordinal.
    pub const fn new(origin: InstallationOrigin, ordinal: u32) -> Self {
        Self { origin, ordinal }
    }

    /// Returns the semantic installation source.
    pub const fn origin(self) -> InstallationOrigin {
        self.origin
    }

    /// Returns the declaration ordinal within the source list.
    pub const fn ordinal(self) -> u32 {
        self.ordinal
    }

    /// Returns whether this installation is available before or during configuration.
    pub const fn phase(self) -> CompositionPhase {
        self.origin.phase()
    }
}

/// The owner of a contribution emitted into the final application plan.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Contributor {
    /// Framework infrastructure contributed by `overseerd-app`.
    Framework,
    /// A direct application declaration.
    Application,
    /// The selected protocol definition.
    Protocol(ProtocolId),
    /// An installed plugin implementation.
    Plugin(PluginId),
}

/// Stable contributor-local provenance for one composition contribution.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContributionProvenance {
    contributor: Contributor,
    contribution: ContributionId,
}

impl ContributionProvenance {
    /// Creates contribution provenance from its owner and local identity.
    pub const fn new(contributor: Contributor, contribution: ContributionId) -> Self {
        Self {
            contributor,
            contribution,
        }
    }

    /// Returns the semantic contribution owner.
    pub const fn contributor(self) -> Contributor {
        self.contributor
    }

    /// Returns the identity local to that owner.
    pub const fn contribution(self) -> ContributionId {
        self.contribution
    }
}

/// The mutation policy for a plugin capability slot.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SlotPolicy {
    /// The installed implementation cannot be replaced or disabled.
    Fixed,
    /// The implementation may be replaced but the slot must remain occupied.
    Replaceable,
    /// The implementation may be replaced or explicitly disabled.
    Optional,
}

/// A target addressed by exact plugin identity or effective capability slot.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RelationTarget {
    /// A specific plugin implementation.
    Plugin(PluginId),
    /// Whichever plugin implementation occupies a slot.
    Slot(PluginSlotId),
}

/// The semantic relationship from one plugin to another target.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RelationKind {
    /// The target must exist and precede the source plugin.
    Requires,
    /// The source and effective target cannot both be installed.
    Conflicts,
    /// The source precedes the target when the target exists.
    Before,
    /// The source follows the target when the target exists.
    After,
}

/// One dependency, conflict, or ordering declaration owned by a plugin.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PluginRelation {
    kind: RelationKind,
    target: RelationTarget,
}

impl PluginRelation {
    /// Creates a relation with explicit semantics and target identity.
    pub const fn new(kind: RelationKind, target: RelationTarget) -> Self {
        Self { kind, target }
    }

    /// Returns the relationship semantics.
    pub const fn kind(self) -> RelationKind {
        self.kind
    }

    /// Returns the exact-plugin or capability-slot target.
    pub const fn target(self) -> RelationTarget {
        self.target
    }
}

/// A plugin implementation and its structural composition metadata.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PluginDeclaration {
    id: PluginId,
    provenance: InstallationProvenance,
    slot: Option<(PluginSlotId, SlotPolicy)>,
    relations: Box<[PluginRelation]>,
}

impl PluginDeclaration {
    /// Creates a plugin declaration without a capability slot or graph relations.
    pub fn new(id: PluginId, provenance: InstallationProvenance) -> Self {
        Self {
            id,
            provenance,
            slot: None,
            relations: Box::new([]),
        }
    }

    /// Assigns the single capability slot provided by this installation.
    pub fn provides(mut self, slot: PluginSlotId, policy: SlotPolicy) -> Self {
        self.slot = Some((slot, policy));

        self
    }

    /// Adds one dependency, conflict, or ordering relation.
    pub fn relates(mut self, relation: PluginRelation) -> Self {
        let mut relations = self.relations.into_vec();

        relations.push(relation);
        self.relations = relations.into_boxed_slice();

        self
    }

    /// Adds several dependency, conflict, or ordering relations.
    pub fn with_relations(mut self, relations: impl IntoIterator<Item = PluginRelation>) -> Self {
        self.relations = relations.into_iter().collect();

        self
    }

    /// Returns the exact implementation identity.
    pub const fn id(&self) -> PluginId {
        self.id
    }

    /// Returns where and when this plugin was declared.
    pub const fn provenance(&self) -> InstallationProvenance {
        self.provenance
    }

    /// Returns the capability slot and policy supplied by this plugin, when present.
    pub const fn slot(&self) -> Option<(PluginSlotId, SlotPolicy)> {
        self.slot
    }

    /// Returns the plugin's structural graph relations.
    pub fn relations(&self) -> &[PluginRelation] {
        &self.relations
    }
}

/// An explicit plugin install, slot replacement, or optional-slot suppression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionDirective {
    kind: DirectiveKind,
}

impl CompositionDirective {
    /// Installs an ordinary plugin declaration.
    pub fn install(plugin: PluginDeclaration) -> Self {
        Self {
            kind: DirectiveKind::Install(plugin),
        }
    }

    /// Replaces the implementation occupying a declared capability slot.
    pub fn replace(slot: PluginSlotId, plugin: PluginDeclaration) -> Self {
        Self {
            kind: DirectiveKind::Replace { slot, plugin },
        }
    }

    /// Disables an optional capability slot.
    pub fn suppress(slot: PluginSlotId, provenance: InstallationProvenance) -> Self {
        Self {
            kind: DirectiveKind::Suppress { slot, provenance },
        }
    }

    pub(crate) fn kind(&self) -> &DirectiveKind {
        &self.kind
    }

    pub(crate) fn provenance(&self) -> InstallationProvenance {
        match &self.kind {
            DirectiveKind::Install(plugin) | DirectiveKind::Replace { plugin, .. } => {
                plugin.provenance()
            }
            DirectiveKind::Suppress { provenance, .. } => *provenance,
        }
    }
}

/// The internal payload of a public composition directive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DirectiveKind {
    /// Installs an ordinary plugin.
    Install(PluginDeclaration),
    /// Replaces an existing slot occupant.
    Replace {
        slot: PluginSlotId,
        plugin: PluginDeclaration,
    },
    /// Disables an optional slot.
    Suppress {
        slot: PluginSlotId,
        provenance: InstallationProvenance,
    },
}
