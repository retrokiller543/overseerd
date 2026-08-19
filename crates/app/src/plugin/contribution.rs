use std::collections::BTreeSet;

#[cfg(feature = "tooling")]
use std::any::TypeId;
#[cfg(feature = "tooling")]
use std::collections::BTreeMap;

use upwell_config::{ConfigBinding, ConfigProperties};
use upwell_core::{Descriptor, NamespacedIdType};
use upwell_di::{ComponentDescriptor, ProviderDescriptor};

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
    #[cfg(feature = "tooling")]
    observations: Vec<PluginContributionObservation>,
    #[cfg(feature = "tooling")]
    reconciliation: BTreeMap<PluginContributionKey, PluginContributionReconciliation>,
    #[cfg(feature = "tooling")]
    tooling: Vec<crate::tooling::ToolingContributionSet>,
    #[cfg(all(feature = "cli", feature = "tooling"))]
    cli_parser_metadata: Option<upwell_tooling_schema::CliMetadata>,
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

    /// Returns validated owner-scoped generic tooling metadata.
    #[cfg(feature = "tooling")]
    pub(crate) fn tooling_contributions(
        &self,
    ) -> impl Iterator<Item = &crate::tooling::ToolingContributionSet> {
        self.tooling.iter()
    }

    #[cfg(all(feature = "cli", feature = "tooling"))]
    pub(crate) const fn cli_parser_metadata(&self) -> Option<&upwell_tooling_schema::CliMetadata> {
        self.cli_parser_metadata.as_ref()
    }

    #[cfg(feature = "tooling")]
    pub(crate) fn reconcile(&mut self, registry: &AppRegistry) {
        self.reconciliation =
            reconcile_contributions(&self.contributions, &self.observations, registry);
    }

    #[cfg(feature = "tooling")]
    pub(crate) fn selected_contribution(&self, target: &str) -> Option<ContributionProvenance> {
        self.contributions
            .iter()
            .copied()
            .enumerate()
            .find_map(|(index, contribution)| {
                let key = PluginContributionKey::new(contribution.provenance(), index);
                let reconciliation = self
                    .reconciliation
                    .get(&key)
                    .expect("every plugin emission has one reconciliation outcome");

                (reconciliation.decision == PluginContributionDecision::Applied
                    && reconciliation.applied.as_deref() == Some(target))
                .then_some(contribution.provenance())
            })
    }

    #[cfg(feature = "tooling")]
    pub(crate) fn reconciled_contributions(
        &self,
    ) -> impl Iterator<Item = (PluginContribution, &PluginContributionReconciliation)> {
        self.contributions
            .iter()
            .copied()
            .enumerate()
            .map(|(index, contribution)| {
                let key = PluginContributionKey::new(contribution.provenance(), index);
                let reconciliation = self
                    .reconciliation
                    .get(&key)
                    .expect("every plugin emission has one reconciliation outcome");

                (contribution, reconciliation)
            })
    }
}

#[cfg(feature = "tooling")]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
/// Internal final-registry outcome for one emitted plugin contribution.
pub(crate) enum PluginContributionDecision {
    Applied,
    Displaced,
    NotApplied,
    Duplicate,
}

#[cfg(feature = "tooling")]
impl PluginContributionDecision {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Displaced => "displaced",
            Self::NotApplied => "not-applied",
            Self::Duplicate => "duplicate",
        }
    }
}

/// A provenance-aware collector supplied to one effective plugin.
pub struct PluginContributions {
    contributor: PluginId,
    contributions: Vec<CollectedContribution>,
    #[cfg(feature = "tooling")]
    tooling: crate::ToolingContributions,
}

impl PluginContributions {
    pub(super) fn new(contributor: PluginId) -> Self {
        Self {
            contributor,
            contributions: Vec::new(),
            #[cfg(feature = "tooling")]
            tooling: crate::ToolingContributions::new(format!("plugin:{}", contributor.as_str())),
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

    /// Returns this plugin's owner-scoped generic tooling metadata collector.
    #[cfg(feature = "tooling")]
    pub fn tooling(&mut self) -> &mut crate::ToolingContributions {
        &mut self.tooling
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

    pub(super) fn finish(self) -> Result<CollectedPluginContributions, PluginPlanError> {
        let mut identities = BTreeSet::new();

        for contribution in &self.contributions {
            let provenance = contribution.metadata.provenance;

            if provenance
                .contribution()
                .is_in_namespace(upwell_core::FRAMEWORK_NAMESPACE)
            {
                return Err(PluginPlanError::ReservedContributionNamespace { provenance });
            }

            if !identities.insert(provenance.contribution()) {
                return Err(PluginPlanError::DuplicateContribution { provenance });
            }
        }

        Ok(CollectedPluginContributions {
            contributions: self.contributions,
            #[cfg(feature = "tooling")]
            tooling: self.tooling.finish()?,
        })
    }
}

/// A typed failure while freezing or lowering retained plugin state.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PluginPlanError {
    /// One plugin declared the same contributor-local CLI provider identity more than once.
    #[cfg(feature = "cli")]
    #[error("plugin CLI provider '{provenance:?}' is declared more than once")]
    DuplicateCliProvider {
        /// The duplicated stable CLI provider provenance.
        provenance: ContributionProvenance,
    },

    /// A third-party plugin attempted to claim a framework-owned CLI provider identity.
    #[cfg(feature = "cli")]
    #[error("plugin CLI provider '{provenance:?}' uses the reserved 'upwell/' namespace")]
    ReservedCliProviderNamespace {
        /// The invalid stable CLI provider provenance.
        provenance: ContributionProvenance,
    },

    /// One plugin emitted the same contributor-local identity more than once.
    #[error("plugin contribution '{provenance:?}' is declared more than once")]
    DuplicateContribution {
        /// The duplicated stable contribution provenance.
        provenance: ContributionProvenance,
    },

    /// A third-party plugin attempted to claim a framework-owned contribution identity.
    #[error("plugin contribution '{provenance:?}' uses the reserved 'upwell/' namespace")]
    ReservedContributionNamespace {
        /// The invalid contribution provenance.
        provenance: ContributionProvenance,
    },

    /// Owner-scoped generic tooling metadata is structurally invalid.
    #[cfg(feature = "tooling")]
    #[error(transparent)]
    Tooling(#[from] crate::ToolingContributionError),

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
    #[cfg(feature = "tooling")]
    pub(super) tooling: Vec<crate::tooling::ToolingContributionSet>,
    #[cfg(all(feature = "cli", feature = "tooling"))]
    pub(super) cli_parser_metadata: Option<upwell_tooling_schema::CliMetadata>,
}

impl CollectedPluginPlan {
    pub(crate) fn lower(self, registry: &mut AppRegistry) -> EffectivePluginPlan {
        let mut metadata = Vec::with_capacity(self.contributions.len());
        #[cfg(feature = "tooling")]
        let mut observations = Vec::with_capacity(self.contributions.len());

        for contribution in self.contributions {
            #[cfg(feature = "tooling")]
            observations.push(PluginContributionObservation::capture(
                &contribution.payload,
                registry,
            ));
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
            #[cfg(feature = "tooling")]
            observations,
            #[cfg(feature = "tooling")]
            reconciliation: BTreeMap::new(),
            #[cfg(feature = "tooling")]
            tooling: self.tooling,
            #[cfg(all(feature = "cli", feature = "tooling"))]
            cli_parser_metadata: self.cli_parser_metadata,
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
/// # use upwell_app::{PluginContributions, contribute};
/// # use upwell_config::ConfigProperties;
/// # use upwell_core::Descriptor;
/// # use upwell_di::{ComponentDescriptor, ProviderDescriptor};
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

/// Collected runtime and tooling emissions from one synchronously consumed plugin.
pub(super) struct CollectedPluginContributions {
    pub(super) contributions: Vec<CollectedContribution>,
    #[cfg(feature = "tooling")]
    pub(super) tooling: crate::tooling::ToolingContributionSet,
}

#[cfg(test)]
impl CollectedPluginContributions {
    pub(super) fn len(&self) -> usize {
        self.contributions.len()
    }
}

impl CollectedContribution {
    #[cfg(feature = "cli")]
    pub(super) const fn provenance(&self) -> ContributionProvenance {
        self.metadata.provenance
    }
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

    #[cfg(feature = "tooling")]
    fn target(&self) -> ContributionTarget {
        match self {
            Self::Component(descriptor) => ContributionTarget::new(
                format!("component:{}", descriptor.id),
                ContributionTargetKey::Component(descriptor.ty.type_id),
            ),
            Self::Provider(descriptor) => ContributionTarget::new(
                provider_target(descriptor),
                ContributionTargetKey::Provider {
                    trait_ty: descriptor.trait_ty.type_id,
                    concrete_ty: descriptor.concrete_ty.type_id,
                    qualifier: descriptor.qualifier.to_string(),
                },
            ),
            Self::ConfigBinding(binding) => ContributionTarget::new(
                format!(
                    "config-binding:{}:{}",
                    (binding.ty.type_name)(),
                    binding.path
                ),
                ContributionTargetKey::ConfigBinding {
                    ty: binding.ty.type_id,
                    path: binding.path.clone(),
                },
            ),
        }
    }

    #[cfg(feature = "tooling")]
    fn exists_in(&self, registry: &AppRegistry) -> bool {
        match self {
            Self::Component(descriptor) => registry.components.iter().any(|existing| {
                existing.ty.type_id == descriptor.ty.type_id && existing.id == descriptor.id
            }),
            Self::Provider(descriptor) => registry.providers.iter().any(|existing| {
                existing.trait_ty.type_id == descriptor.trait_ty.type_id
                    && existing.concrete_ty.type_id == descriptor.concrete_ty.type_id
                    && existing.qualifier == descriptor.qualifier
            }),
            Self::ConfigBinding(binding) => registry.config_bindings.iter().any(|existing| {
                existing.ty.type_id == binding.ty.type_id && existing.path == binding.path
            }),
        }
    }
}

#[cfg(feature = "tooling")]
#[derive(Clone, Debug, Eq, PartialEq)]
/// Pre-lowering target observation retained only to interpret the final registry.
struct PluginContributionObservation {
    requested: String,
    key: ContributionTargetKey,
    preceded: bool,
}

#[cfg(feature = "tooling")]
impl PluginContributionObservation {
    fn capture(payload: &ContributionPayload, registry: &AppRegistry) -> Self {
        let target = payload.target();

        Self {
            requested: target.requested,
            key: target.key,
            preceded: payload.exists_in(registry),
        }
    }
}

#[cfg(feature = "tooling")]
#[derive(Clone, Debug, Eq, PartialEq)]
/// Final-registry reconciliation for one provenance-and-index keyed plugin emission.
pub(crate) struct PluginContributionReconciliation {
    pub(crate) requested: String,
    pub(crate) applied: Option<String>,
    pub(crate) decision: PluginContributionDecision,
}

#[cfg(feature = "tooling")]
impl PluginContributionObservation {
    fn final_target(&self, registry: &AppRegistry) -> Option<String> {
        match &self.key {
            ContributionTargetKey::Component(ty) => registry
                .components
                .iter()
                .find(|component| component.ty.type_id == *ty)
                .map(|component| format!("component:{}", component.id)),
            ContributionTargetKey::Provider {
                trait_ty,
                concrete_ty,
                qualifier,
            } => registry
                .providers
                .iter()
                .find(|provider| {
                    provider.trait_ty.type_id == *trait_ty
                        && provider.concrete_ty.type_id == *concrete_ty
                        && provider.qualifier == qualifier
                })
                .map(provider_target),
            ContributionTargetKey::ConfigBinding { ty, path } => registry
                .config_bindings
                .iter()
                .find(|binding| binding.ty.type_id == *ty && binding.path == *path)
                .map(config_binding_target),
        }
    }
}

#[cfg(feature = "tooling")]
fn reconcile_contributions(
    contributions: &[PluginContribution],
    observations: &[PluginContributionObservation],
    registry: &AppRegistry,
) -> BTreeMap<PluginContributionKey, PluginContributionReconciliation> {
    let mut selected = BTreeSet::new();
    let mut reconciliations = BTreeMap::new();

    for (index, (contribution, observation)) in
        contributions.iter().copied().zip(observations).enumerate()
    {
        let key = PluginContributionKey::new(contribution.provenance(), index);
        let final_target = observation.final_target(registry);
        let selected_key = final_target
            .as_ref()
            .map(|applied| (observation.key.clone(), applied.clone()));

        let decision = match (&final_target, selected_key) {
            (None, _) => PluginContributionDecision::NotApplied,
            (Some(applied), _) if observation.preceded && applied == &observation.requested => {
                PluginContributionDecision::Duplicate
            }
            (Some(_), Some(key)) if !selected.insert(key.clone()) => {
                PluginContributionDecision::Duplicate
            }
            (Some(applied), _) if applied == &observation.requested => {
                PluginContributionDecision::Applied
            }
            (Some(_), _) => PluginContributionDecision::Displaced,
        };

        reconciliations.insert(
            key,
            PluginContributionReconciliation {
                requested: observation.requested.clone(),
                applied: final_target,
                decision,
            },
        );
    }

    reconciliations
}

#[cfg(feature = "tooling")]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
/// Stable private key for one contributor-local emission occurrence.
struct PluginContributionKey {
    provenance: ContributionProvenance,
    index: usize,
}

#[cfg(feature = "tooling")]
impl PluginContributionKey {
    const fn new(provenance: ContributionProvenance, index: usize) -> Self {
        Self { provenance, index }
    }
}

#[cfg(feature = "tooling")]
/// Tooling-only requested resource identity and lookup key.
struct ContributionTarget {
    requested: String,
    key: ContributionTargetKey,
}

#[cfg(feature = "tooling")]
impl ContributionTarget {
    fn new(requested: String, key: ContributionTargetKey) -> Self {
        Self { requested, key }
    }
}

#[cfg(feature = "tooling")]
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
/// Stable registry lookup key for an emitted resource category.
enum ContributionTargetKey {
    Component(TypeId),
    Provider {
        trait_ty: TypeId,
        concrete_ty: TypeId,
        qualifier: String,
    },
    ConfigBinding {
        ty: TypeId,
        path: String,
    },
}

#[cfg(feature = "tooling")]
fn provider_target(descriptor: &ProviderDescriptor) -> String {
    format!(
        "provider:{}:{}:{}",
        (descriptor.trait_ty.type_name)(),
        (descriptor.concrete_ty.type_name)(),
        descriptor.qualifier
    )
}

#[cfg(feature = "tooling")]
fn config_binding_target(binding: &ConfigBinding) -> String {
    format!(
        "config-binding:{}:{}",
        (binding.ty.type_name)(),
        binding.path
    )
}
