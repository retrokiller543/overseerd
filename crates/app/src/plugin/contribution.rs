use std::collections::BTreeSet;

use overseerd_config::{ConfigBinding, ConfigProperties};
use overseerd_core::{Descriptor, NamespacedIdType};
use overseerd_di::{ComponentDescriptor, ProviderDescriptor};

use crate::{
    AppRegistry, ContributionId, ContributionProvenance, Contributor, InstallationProvenance,
    PluginId, PluginResolutionPlan,
};

/// The app-neutral category of one attributed plugin contribution.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum PluginContributionKind {
    /// A component descriptor contributed to dependency injection.
    Component,
    /// A trait-provider descriptor contributed to dependency injection.
    Provider,
    /// A configuration binding contributed before config resolution.
    ConfigBinding,
}

/// Public immutable metadata for one effective plugin contribution.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PluginContribution {
    provenance: ContributionProvenance,
    kind: PluginContributionKind,
}

impl PluginContribution {
    /// Returns the stable contributor and contributor-local contribution identity.
    pub const fn provenance(self) -> ContributionProvenance {
        self.provenance
    }

    /// Returns the app-neutral contribution category.
    pub const fn kind(self) -> PluginContributionKind {
        self.kind
    }
}

/// The immutable plugin resolution and attributed emissions from its effective plugins.
///
/// Later direct protocol registration may override a lowered descriptor until protocol-owned
/// contributions migrate into this plan. This plan remains the authoritative record of plugin
/// selection and plugin emissions, not yet the final effective application registry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectivePluginPlan {
    resolution: PluginResolutionPlan,
    contributions: Box<[PluginContribution]>,
}

impl EffectivePluginPlan {
    /// Returns the deterministic structural plugin resolution.
    pub const fn resolution(&self) -> &PluginResolutionPlan {
        &self.resolution
    }

    /// Returns plugin emissions in resolution and contributor-local emission order.
    pub fn emitted_contributions(&self) -> &[PluginContribution] {
        &self.contributions
    }
}

/// A provenance-aware collector supplied to one effective plugin.
pub struct PluginContributions {
    contributor: PluginId,
    contributions: Vec<CollectedContribution>,
}

impl PluginContributions {
    pub(super) fn new(contributor: PluginId) -> Self {
        Self {
            contributor,
            contributions: Vec::new(),
        }
    }

    /// Contributes a raw component descriptor.
    pub fn component(&mut self, id: ContributionId, descriptor: ComponentDescriptor) {
        self.push(id, ContributionPayload::Component(descriptor));
    }

    /// Contributes component type `T` through its static descriptor.
    pub fn component_type<T>(&mut self, id: ContributionId)
    where
        T: Descriptor<ComponentDescriptor>,
    {
        self.component(id, <T as Descriptor<ComponentDescriptor>>::DESCRIPTOR);
    }

    /// Contributes a trait-provider descriptor.
    pub fn provider(&mut self, id: ContributionId, descriptor: ProviderDescriptor) {
        self.push(id, ContributionPayload::Provider(descriptor));
    }

    /// Contributes a configuration binding.
    pub fn config<T: ConfigProperties>(&mut self, id: ContributionId, path: impl Into<String>) {
        self.push(
            id,
            ContributionPayload::ConfigBinding(ConfigBinding::of::<T>(path)),
        );
    }

    fn push(&mut self, id: ContributionId, payload: ContributionPayload) {
        let provenance = ContributionProvenance::new(Contributor::Plugin(self.contributor), id);
        let metadata = PluginContribution {
            provenance,
            kind: payload.kind(),
        };

        self.contributions
            .push(CollectedContribution { metadata, payload });
    }

    pub(super) fn finish(self) -> Result<Vec<CollectedContribution>, PluginPlanError> {
        let mut identities = BTreeSet::new();

        for contribution in &self.contributions {
            let provenance = contribution.metadata.provenance;

            if provenance
                .contribution()
                .is_in_namespace(overseerd_core::FRAMEWORK_NAMESPACE)
            {
                return Err(PluginPlanError::ReservedContributionNamespace { provenance });
            }

            if !identities.insert(provenance.contribution()) {
                return Err(PluginPlanError::DuplicateContribution { provenance });
            }
        }

        Ok(self.contributions)
    }
}

/// A typed failure while freezing or lowering retained plugin state.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PluginPlanError {
    /// One plugin emitted the same contributor-local identity more than once.
    #[error("plugin contribution '{provenance:?}' is declared more than once")]
    DuplicateContribution {
        /// The duplicated stable contribution provenance.
        provenance: ContributionProvenance,
    },

    /// A third-party plugin attempted to claim a framework-owned contribution identity.
    #[error("plugin contribution '{provenance:?}' uses the reserved 'overseerd/' namespace")]
    ReservedContributionNamespace {
        /// The invalid contribution provenance.
        provenance: ContributionProvenance,
    },

    /// The structural resolver selected a plugin whose retained payload is absent.
    #[error("resolved plugin '{plugin}' has no retained installation ({provenance:?})")]
    MissingInstallation {
        /// The selected stable plugin identity.
        plugin: PluginId,
        /// The selected installation provenance.
        provenance: InstallationProvenance,
    },
}

pub(crate) struct CollectedPluginPlan {
    pub(super) resolution: PluginResolutionPlan,
    pub(super) contributions: Vec<CollectedContribution>,
}

impl CollectedPluginPlan {
    pub(crate) fn lower(self, registry: &mut AppRegistry) -> EffectivePluginPlan {
        let mut metadata = Vec::with_capacity(self.contributions.len());

        for contribution in self.contributions {
            metadata.push(contribution.metadata);

            match contribution.payload {
                ContributionPayload::Component(descriptor) => {
                    registry.components.push(descriptor);
                }
                ContributionPayload::Provider(descriptor) => {
                    registry.providers.push(descriptor);
                }
                ContributionPayload::ConfigBinding(binding) => {
                    registry.config_bindings.push(binding);
                }
            }
        }

        EffectivePluginPlan {
            resolution: self.resolution,
            contributions: metadata.into_boxed_slice(),
        }
    }
}

pub(super) struct CollectedContribution {
    metadata: PluginContribution,
    payload: ContributionPayload,
}

enum ContributionPayload {
    Component(ComponentDescriptor),
    Provider(ProviderDescriptor),
    ConfigBinding(ConfigBinding),
}

impl ContributionPayload {
    const fn kind(&self) -> PluginContributionKind {
        match self {
            Self::Component(_) => PluginContributionKind::Component,
            Self::Provider(_) => PluginContributionKind::Provider,
            Self::ConfigBinding(_) => PluginContributionKind::ConfigBinding,
        }
    }
}
