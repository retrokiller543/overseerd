use crate::composition::{
    CompositionDirective, EarlyPluginPlan, InstallationOrigin, InstallationProvenance,
    PluginDeclaration, PluginSlotId, ResolvedPlugin, SlotPolicy, extend_late_plugins,
    resolve_early_plugins,
};
use crate::{PluginId, ProtocolDefinition, ProtocolId};

#[cfg(feature = "cli")]
use std::collections::BTreeSet;
#[cfg(feature = "cli")]
use upwell_core::NamespacedIdType;

use super::Plugin;
use super::contribution::{CollectedPluginPlan, PluginContributions, PluginPlanError};

#[cfg(feature = "cli")]
use super::contribution::CollectedContribution;

#[cfg(feature = "cli")]
use crate::{
    CliDefinitionError, ParsedPluginArgs, PluginCliProviderMetadata, PluginCliRegistrar,
    SelectedPluginCliCommand,
    host::{PluginCliProvider, augment_plugin_cli},
};

/// A protocol-owned registrar for mandatory and default plugin installations.
pub struct ProtocolPluginRegistrar {
    protocol: ProtocolId,
    installations: Vec<RetainedPlugin>,
    mandatory_ordinal: u32,
    default_ordinal: u32,
}

impl ProtocolPluginRegistrar {
    pub(crate) const fn new(protocol: ProtocolId) -> Self {
        Self {
            protocol,
            installations: Vec::new(),
            mandatory_ordinal: 0,
            default_ordinal: 0,
        }
    }

    /// Declares one mandatory protocol-owned plugin from a supplied instance.
    pub fn mandatory<P: Plugin>(&mut self, plugin: P) {
        let provenance = InstallationProvenance::new(
            InstallationOrigin::ProtocolMandatory(self.protocol),
            self.mandatory_ordinal,
        );

        self.mandatory_ordinal = next_ordinal(self.mandatory_ordinal);

        self.installations
            .push(RetainedPlugin::install(plugin, provenance, None));
    }

    /// Declares one replaceable protocol default from a supplied instance.
    pub fn replaceable_default<P: Plugin>(&mut self, slot: PluginSlotId, plugin: P) {
        self.default(plugin, slot, SlotPolicy::Replaceable);
    }

    /// Declares one suppressible protocol default from a supplied instance.
    pub fn optional_default<P: Plugin>(&mut self, slot: PluginSlotId, plugin: P) {
        self.default(plugin, slot, SlotPolicy::Optional);
    }

    fn default<P: Plugin>(&mut self, plugin: P, slot: PluginSlotId, policy: SlotPolicy) {
        let provenance = InstallationProvenance::new(
            InstallationOrigin::ProtocolDefault(self.protocol),
            self.default_ordinal,
        );

        self.default_ordinal = next_ordinal(self.default_ordinal);

        self.installations.push(RetainedPlugin::install(
            plugin,
            provenance,
            Some((slot, policy)),
        ));
    }
}

/// Static application plugin declarations resolved before dynamic builder configuration.
pub struct ApplicationPluginRegistrar {
    directives: Vec<RetainedDirective>,
}

impl ApplicationPluginRegistrar {
    pub(crate) const fn new() -> Self {
        Self {
            directives: Vec::new(),
        }
    }

    /// Declares an early application plugin by type using synchronous default construction.
    pub fn register<P: Plugin + Default>(&mut self) {
        self.with_plugin(P::default());
    }

    /// Declares an early application plugin from a supplied instance.
    pub fn with_plugin<P: Plugin>(&mut self, plugin: P) {
        let provenance = self.provenance();

        self.directives
            .push(RetainedDirective::Plugin(RetainedPlugin::install(
                plugin, provenance, None,
            )));
    }

    /// Replaces a protocol default with a supplied plugin instance.
    pub fn replace<P: Plugin>(&mut self, slot: PluginSlotId, plugin: P) {
        let provenance = self.provenance();

        self.directives
            .push(RetainedDirective::Plugin(RetainedPlugin::replace(
                plugin, provenance, slot,
            )));
    }

    /// Replaces a protocol default by plugin type using synchronous default construction.
    pub fn replace_with<P: Plugin + Default>(&mut self, slot: PluginSlotId) {
        self.replace(slot, P::default());
    }

    /// Explicitly disables an optional protocol-default slot.
    pub fn suppress(&mut self, slot: PluginSlotId) {
        let provenance = self.provenance();

        self.directives
            .push(RetainedDirective::Suppression { slot, provenance });
    }

    fn provenance(&self) -> InstallationProvenance {
        InstallationProvenance::new(
            InstallationOrigin::ApplicationDeclaration,
            ordinal(self.directives.len()),
        )
    }
}

pub(crate) struct PluginCatalog {
    early: ApplicationPluginRegistrar,
    resolved: Option<EarlyPluginCatalog>,
    late: Vec<RetainedPlugin>,
}

impl PluginCatalog {
    pub(crate) const fn new() -> Self {
        Self {
            early: ApplicationPluginRegistrar::new(),
            resolved: None,
            late: Vec::new(),
        }
    }

    pub(crate) fn use_early(&mut self, catalog: EarlyPluginCatalog) {
        self.resolved = Some(catalog);
    }

    pub(crate) fn declare(&mut self, declarations: impl FnOnce(&mut ApplicationPluginRegistrar)) {
        declarations(&mut self.early);
    }

    pub(crate) fn with_plugin<P: Plugin>(&mut self, plugin: P) {
        let provenance = InstallationProvenance::new(
            InstallationOrigin::ApplicationConfiguration,
            ordinal(self.late.len()),
        );

        self.late
            .push(RetainedPlugin::install(plugin, provenance, None));
    }

    pub(crate) fn freeze<D: ProtocolDefinition>(
        self,
        discover: bool,
    ) -> crate::Result<CollectedPluginPlan> {
        let early = match self.resolved {
            Some(early) => early,
            None => {
                let mut protocol = ProtocolPluginRegistrar::new(D::ID);

                D::register_plugins(&mut protocol);

                EarlyPluginCatalog::resolve(protocol, self.early)?
            }
        };
        let resolution = extend_late_plugins(
            early.plan(),
            self.late.iter().map(RetainedPlugin::directive),
        )?;
        #[cfg(feature = "cli")]
        let cli_metadata = early.cli_provider_metadata().to_vec();
        #[cfg(all(feature = "cli", feature = "tooling"))]
        let cli_parser_metadata = early.cli_parser_metadata.clone();
        let mut installations: Vec<_> = early
            .into_installations()
            .chain(self.late)
            .map(Some)
            .collect();
        let mut contributions = Vec::new();
        #[cfg(feature = "tooling")]
        let mut tooling = Vec::new();

        for plugin in resolution.plugins() {
            let index = installations
                .iter()
                .position(|installation| {
                    installation
                        .as_ref()
                        .is_some_and(|installation| installation.matches(plugin))
                })
                .ok_or(PluginPlanError::MissingInstallation {
                    plugin: plugin.id(),
                    provenance: plugin.provenance(),
                })?;
            let installation =
                installations[index]
                    .take()
                    .ok_or(PluginPlanError::MissingInstallation {
                        plugin: plugin.id(),
                        provenance: plugin.provenance(),
                    })?;

            let plugin_contributions = installation.collect(discover)?;

            #[cfg(feature = "cli")]
            validate_contribution_identities(&cli_metadata, &plugin_contributions.contributions)?;

            contributions.extend(plugin_contributions.contributions);
            #[cfg(feature = "tooling")]
            tooling.push(plugin_contributions.tooling);
        }

        Ok(CollectedPluginPlan {
            resolution,
            contributions,
            #[cfg(feature = "tooling")]
            tooling,
            #[cfg(all(feature = "cli", feature = "tooling"))]
            cli_parser_metadata,
        })
    }
}

/// A resolved parser-visible plugin catalog retaining the selected plugin instances.
///
/// The catalog is move-only. Generated CLI bootstrap resolves it before parser construction and
/// transfers the same instances into application preparation, where discovery and contribution
/// collection consume them exactly once.
#[doc(hidden)]
pub struct EarlyPluginCatalog {
    plan: EarlyPluginPlan,
    installations: Vec<RetainedPlugin>,
    #[cfg(feature = "cli")]
    cli_providers: Vec<PluginCliProvider>,
    #[cfg(feature = "cli")]
    cli_metadata: Vec<PluginCliProviderMetadata>,
    #[cfg(all(feature = "cli", feature = "tooling"))]
    cli_parser_metadata: Option<upwell_tooling_schema::CliMetadata>,
}

impl EarlyPluginCatalog {
    pub(crate) fn resolve(
        protocol: ProtocolPluginRegistrar,
        application: ApplicationPluginRegistrar,
    ) -> crate::Result<Self> {
        let directives: Vec<_> = protocol
            .installations
            .iter()
            .map(RetainedPlugin::directive)
            .chain(
                application
                    .directives
                    .iter()
                    .map(RetainedDirective::directive),
            )
            .collect();
        let plan = resolve_early_plugins(protocol.protocol, directives)?;
        let mut candidates: Vec<_> = protocol
            .installations
            .into_iter()
            .chain(
                application
                    .directives
                    .into_iter()
                    .filter_map(RetainedDirective::plugin),
            )
            .map(Some)
            .collect();
        let mut installations = Vec::with_capacity(plan.plugins().len());

        for plugin in plan.plugins() {
            let index = candidates
                .iter()
                .position(|candidate| {
                    candidate
                        .as_ref()
                        .is_some_and(|candidate| candidate.matches(plugin))
                })
                .ok_or(PluginPlanError::MissingInstallation {
                    plugin: plugin.id(),
                    provenance: plugin.provenance(),
                })?;
            let installation =
                candidates[index]
                    .take()
                    .ok_or(PluginPlanError::MissingInstallation {
                        plugin: plugin.id(),
                        provenance: plugin.provenance(),
                    })?;

            installations.push(installation);
        }

        #[cfg(feature = "cli")]
        let cli_providers: Vec<_> = installations.iter().flat_map(RetainedPlugin::cli).collect();
        #[cfg(feature = "cli")]
        let cli_metadata = cli_providers
            .iter()
            .map(PluginCliProvider::metadata)
            .collect::<Vec<_>>();

        #[cfg(feature = "cli")]
        validate_cli_metadata(&cli_metadata)?;

        Ok(Self {
            plan,
            installations,
            #[cfg(feature = "cli")]
            cli_providers,
            #[cfg(feature = "cli")]
            cli_metadata,
            #[cfg(all(feature = "cli", feature = "tooling"))]
            cli_parser_metadata: None,
        })
    }

    /// The deterministic parser-visible plugin resolution.
    pub fn plan(&self) -> &EarlyPluginPlan {
        &self.plan
    }

    /// Deterministic effective CLI provider metadata in plugin resolution order.
    #[cfg(feature = "cli")]
    pub(crate) fn cli_provider_metadata(&self) -> &[PluginCliProviderMetadata] {
        &self.cli_metadata
    }

    /// Adds every effective plugin CLI facet to one generated Clap tree.
    #[cfg(feature = "cli")]
    #[doc(hidden)]
    pub fn augment_cli(
        &mut self,
        command: clap::Command,
        framework: clap::Command,
        application_args: &[std::any::TypeId],
        serve_default: bool,
    ) -> Result<clap::Command, CliDefinitionError> {
        let augmented = augment_plugin_cli(
            command,
            framework,
            &self.cli_providers,
            application_args,
            serve_default,
        )?;

        #[cfg(feature = "tooling")]
        let metadata = augmented.metadata;
        let command = augmented.command;

        #[cfg(feature = "tooling")]
        {
            self.cli_parser_metadata = Some(metadata);
        }

        Ok(command)
    }

    /// Extracts all effective plugin global argument groups from parsed matches.
    #[cfg(feature = "cli")]
    #[doc(hidden)]
    pub fn parse_cli_args(
        &self,
        matches: &mut clap::ArgMatches,
    ) -> Result<ParsedPluginArgs, clap::Error> {
        let mut values = Vec::new();

        for provider in &self.cli_providers {
            if let Some(value) = provider.extract_args(matches)? {
                values.push(value);
            }
        }

        Ok(ParsedPluginArgs::new(values))
    }

    /// Extracts the selected plugin command, when the parsed subcommand belongs to a provider.
    #[cfg(feature = "cli")]
    #[doc(hidden)]
    pub fn parse_cli_command(
        &self,
        matches: &mut clap::ArgMatches,
    ) -> Result<Option<SelectedPluginCliCommand>, clap::Error> {
        let Some(name) = matches.subcommand_name().map(str::to_owned) else {
            return Ok(None);
        };

        for provider in &self.cli_providers {
            if provider.matches_command(&name) {
                return provider.extract_command(matches);
            }
        }

        Ok(None)
    }

    fn into_installations(self) -> impl Iterator<Item = RetainedPlugin> {
        self.installations.into_iter()
    }
}

#[cfg(feature = "cli")]
fn validate_contribution_identities(
    cli: &[PluginCliProviderMetadata],
    contributions: &[CollectedContribution],
) -> Result<(), PluginPlanError> {
    for provider in cli {
        let provenance = provider.provenance();

        if contributions
            .iter()
            .any(|contribution| contribution.provenance() == provenance)
        {
            return Err(PluginPlanError::DuplicateContribution { provenance });
        }
    }

    Ok(())
}

#[cfg(feature = "cli")]
fn validate_cli_metadata(metadata: &[PluginCliProviderMetadata]) -> Result<(), PluginPlanError> {
    let mut identities = BTreeSet::new();

    for provider in metadata {
        let provenance = provider.provenance();

        if provenance
            .contribution()
            .is_in_namespace(upwell_core::FRAMEWORK_NAMESPACE)
        {
            return Err(PluginPlanError::ReservedCliProviderNamespace { provenance });
        }

        if !identities.insert(provenance) {
            return Err(PluginPlanError::DuplicateCliProvider { provenance });
        }
    }

    Ok(())
}

struct RetainedPlugin {
    directive: CompositionDirective,
    declaration: PluginDeclaration,
    plugin: Box<dyn ErasedPlugin>,
}

impl RetainedPlugin {
    fn install<P: Plugin>(
        plugin: P,
        provenance: InstallationProvenance,
        slot: Option<(PluginSlotId, SlotPolicy)>,
    ) -> Self {
        let mut declaration =
            PluginDeclaration::new(P::ID, provenance).with_relations(P::RELATIONS.iter().copied());

        if let Some((slot, policy)) = slot {
            declaration = declaration.provides(slot, policy);
        }

        let directive = CompositionDirective::install(declaration.clone());

        Self {
            directive,
            declaration,
            plugin: Box::new(plugin),
        }
    }

    fn replace<P: Plugin>(
        plugin: P,
        provenance: InstallationProvenance,
        slot: PluginSlotId,
    ) -> Self {
        let declaration =
            PluginDeclaration::new(P::ID, provenance).with_relations(P::RELATIONS.iter().copied());
        let directive = CompositionDirective::replace(slot, declaration.clone());

        Self {
            directive,
            declaration,
            plugin: Box::new(plugin),
        }
    }

    fn directive(&self) -> CompositionDirective {
        self.directive.clone()
    }

    fn matches(&self, plugin: &ResolvedPlugin) -> bool {
        self.declaration.id() == plugin.id() && self.declaration.provenance() == plugin.provenance()
    }

    fn collect(
        self,
        discover: bool,
    ) -> Result<super::contribution::CollectedPluginContributions, PluginPlanError> {
        self.plugin.collect(self.declaration.id(), discover)
    }

    #[cfg(feature = "cli")]
    fn cli(&self) -> Vec<PluginCliProvider> {
        self.plugin.cli(self.declaration.id())
    }
}

enum RetainedDirective {
    Plugin(RetainedPlugin),
    Suppression {
        slot: PluginSlotId,
        provenance: InstallationProvenance,
    },
}

impl RetainedDirective {
    fn directive(&self) -> CompositionDirective {
        match self {
            Self::Plugin(plugin) => plugin.directive(),
            Self::Suppression { slot, provenance } => {
                CompositionDirective::suppress(*slot, *provenance)
            }
        }
    }

    fn plugin(self) -> Option<RetainedPlugin> {
        match self {
            Self::Plugin(plugin) => Some(plugin),
            Self::Suppression { .. } => None,
        }
    }
}

trait ErasedPlugin: Send {
    fn collect(
        self: Box<Self>,
        contributor: PluginId,
        discover: bool,
    ) -> Result<super::contribution::CollectedPluginContributions, PluginPlanError>;

    #[cfg(feature = "cli")]
    fn cli(&self, contributor: PluginId) -> Vec<PluginCliProvider>;
}

impl<P: Plugin> ErasedPlugin for P {
    fn collect(
        mut self: Box<Self>,
        contributor: PluginId,
        discover: bool,
    ) -> Result<super::contribution::CollectedPluginContributions, PluginPlanError> {
        if discover {
            self.auto_discover();
        }

        let mut contributions = PluginContributions::new(contributor);

        (*self).contribute(&mut contributions);

        contributions.finish()
    }

    #[cfg(feature = "cli")]
    fn cli(&self, contributor: PluginId) -> Vec<PluginCliProvider> {
        let mut registrar = PluginCliRegistrar::new(contributor);

        Plugin::cli(self, &mut registrar);

        registrar.finish()
    }
}

fn ordinal(len: usize) -> u32 {
    u32::try_from(len).unwrap_or_else(|_| ordinal_overflow())
}

fn next_ordinal(ordinal: u32) -> u32 {
    ordinal.checked_add(1).unwrap_or_else(|| ordinal_overflow())
}

#[cold]
#[inline(never)]
fn ordinal_overflow() -> ! {
    panic!(
        "plugin installation count exceeds u32::MAX; report this limit with `gh issue create \
         --repo retrokiller543/upwell --title 'Widen plugin installation ordinals' \
         --body 'Plugin installation provenance exceeded u32::MAX; migrate the ordinal to u64 \
         or usize.'`"
    )
}
