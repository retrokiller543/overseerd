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
    contributions: Vec<PluginContribution>,
}

impl EffectivePluginPlan {
    /// Returns the deterministic structural plugin resolution.
    pub const fn resolution(&self) -> &PluginResolutionPlan {
        &self.resolution
    }

    /// Returns plugin emissions in resolution and contributor-local emission order.
    pub const fn emitted_contributions(&self) -> &[PluginContribution] {
        self.contributions.as_slice()
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

    /// Contributes component type `T` through its static descriptor.
    pub fn component<T>(&mut self, id: ContributionId)
    where
        T: Descriptor<ComponentDescriptor>,
    {
        self.component_descriptor(id, <T as Descriptor<ComponentDescriptor>>::DESCRIPTOR);
    }

    /// Contributes a raw component descriptor when no typed descriptor implementation exists.
    pub fn component_descriptor(&mut self, id: ContributionId, descriptor: ComponentDescriptor) {
        self.push(id, ContributionPayload::Component(descriptor));
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
            contributions: metadata,
        }
    }
}

/// Emits explicit app-neutral plugin contributions with stable namespaced identities.
///
/// The macro takes an explicit mutable [`PluginContributions`] target followed by any supported
/// sections in the order shown below. Every left-hand string is a stable, namespaced
/// [`ContributionId`], not the ID of the component or provider payload.
///
/// ```text
/// contribute! {
///     to <collector expression>,
///     components: [
///         "<contribution id>" => type <component type>,
///         "<contribution id>" => <ComponentDescriptor expression>,
///     ],
///     providers: [
///         "<contribution id>" => <ProviderDescriptor expression>,
///     ],
///     configs: [
///         "<contribution id>" => <ConfigProperties type> => <config path expression>,
///     ],
/// }
/// ```
///
/// Each section is optional, but sections that are present must follow that order. Entries and
/// sections may have trailing commas.
///
/// In `components`, `type T` is the preferred form and requires `T` to implement
/// `Descriptor<ComponentDescriptor>`. An expression is the explicit escape hatch for a dynamically
/// assembled descriptor or a component that cannot implement `Descriptor`. The `type` marker is
/// required because a bare path can be valid in both Rust's type and expression grammars, while
/// `type T` is unambiguously a typed contribution in this macro.
///
/// ```
/// # use overseerd_app::{PluginContributions, contribute};
/// # use overseerd_config::ConfigProperties;
/// # use overseerd_core::Descriptor;
/// # use overseerd_di::{ComponentDescriptor, ProviderDescriptor};
/// # struct Scheduler;
/// # #[derive(serde::Deserialize)]
/// # struct JobsConfig;
/// # impl ConfigProperties for JobsConfig { const NAME: &'static str = "JobsConfig"; }
/// # impl Descriptor<ComponentDescriptor> for Scheduler {
/// #     const DESCRIPTOR: ComponentDescriptor = panic!("documentation-only descriptor");
/// # }
/// # fn provider_descriptor() -> ProviderDescriptor { unimplemented!() }
/// # fn add(contributions: &mut PluginContributions) {
/// contribute! {
///     to contributions,
///     components: [
///         "jobs/scheduler-component" => type Scheduler,
///     ],
///     providers: [
///         "jobs/scheduler-provider" => provider_descriptor(),
///     ],
///     configs: [
///         "jobs/config" => JobsConfig => "jobs",
///     ],
/// }
/// # }
/// ```
///
/// Contribution IDs remain explicit because they identify plugin emissions and their provenance.
/// They are intentionally not inferred from descriptor IDs: separate plugins may emit the same
/// descriptor, and one plugin may emit multiple independently identified contributions.
#[macro_export]
macro_rules! contribute {
    (
        to $contributions:expr
        $(, components: [$($components:tt)*])?
        $(, providers: [
            $(
                $provider_id:literal => $provider:expr
            ),* $(,)?
        ])?
        $(, configs: [
            $(
                $config_id:literal => $config:ty => $path:expr
            ),* $(,)?
        ])?
        $(,)?
    ) => {{
        let __contributions: &mut $crate::PluginContributions = $contributions;

        $(
            $crate::contribute!(@components __contributions; $($components)*);
        )?

        $(
            $(
                __contributions.provider(
                    $crate::namespaced_id!($crate::ContributionId, $provider_id),
                    $provider,
                );
            )*
        )?

        $(
            $(
                __contributions.config::<$config>(
                    $crate::namespaced_id!($crate::ContributionId, $config_id),
                    $path,
                );
            )*
        )?
    }};

    (@components $contributions:ident;) => {};

    (@components
        $contributions:ident;
        $id:literal => type $component:ty,
        $($remaining:tt)*
    ) => {
        $contributions.component::<$component>(
            $crate::namespaced_id!($crate::ContributionId, $id),
        );
        $crate::contribute!(@components $contributions; $($remaining)*);
    };

    (@components
        $contributions:ident;
        $id:literal => type $component:ty
    ) => {
        $contributions.component::<$component>(
            $crate::namespaced_id!($crate::ContributionId, $id),
        );
    };

    (@components
        $contributions:ident;
        $id:literal => $descriptor:expr,
        $($remaining:tt)*
    ) => {
        $contributions.component_descriptor(
            $crate::namespaced_id!($crate::ContributionId, $id),
            $descriptor,
        );
        $crate::contribute!(@components $contributions; $($remaining)*);
    };

    (@components
        $contributions:ident;
        $id:literal => $descriptor:expr
    ) => {
        $contributions.component_descriptor(
            $crate::namespaced_id!($crate::ContributionId, $id),
            $descriptor,
        );
    };
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
