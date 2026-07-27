use crate::composition::{
    CompositionDirective, InstallationOrigin, InstallationProvenance, PluginDeclaration,
    PluginSlotId, ResolvedPlugin, SlotPolicy, extend_late_plugins, resolve_early_plugins,
};
use crate::{PluginId, ProtocolId};

use super::Plugin;
use super::contribution::{
    CollectedContribution, CollectedPluginPlan, PluginContributions, PluginPlanError,
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
    late: Vec<RetainedPlugin>,
}

impl PluginCatalog {
    pub(crate) const fn new() -> Self {
        Self {
            early: ApplicationPluginRegistrar::new(),
            late: Vec::new(),
        }
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

    pub(crate) fn freeze(
        self,
        protocol: ProtocolPluginRegistrar,
        discover: bool,
    ) -> crate::Result<CollectedPluginPlan> {
        let early_directives: Vec<_> = protocol
            .installations
            .iter()
            .map(RetainedPlugin::directive)
            .chain(
                self.early
                    .directives
                    .iter()
                    .map(RetainedDirective::directive),
            )
            .collect();
        let early = resolve_early_plugins(protocol.protocol, early_directives)?;
        let resolution =
            extend_late_plugins(&early, self.late.iter().map(RetainedPlugin::directive))?;
        let mut installations: Vec<_> = protocol
            .installations
            .into_iter()
            .chain(
                self.early
                    .directives
                    .into_iter()
                    .filter_map(RetainedDirective::plugin),
            )
            .chain(self.late)
            .map(Some)
            .collect();
        let mut contributions = Vec::new();

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

            contributions.extend(installation.collect(discover)?);
        }

        Ok(CollectedPluginPlan {
            resolution,
            contributions,
        })
    }
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

    fn collect(self, discover: bool) -> Result<Vec<CollectedContribution>, PluginPlanError> {
        self.plugin.collect(self.declaration.id(), discover)
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
    ) -> Result<Vec<CollectedContribution>, PluginPlanError>;
}

impl<P: Plugin> ErasedPlugin for P {
    fn collect(
        mut self: Box<Self>,
        contributor: PluginId,
        discover: bool,
    ) -> Result<Vec<CollectedContribution>, PluginPlanError> {
        if discover {
            self.auto_discover();
        }

        let mut contributions = PluginContributions::new(contributor);

        (*self).contribute(&mut contributions);

        contributions.finish()
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
         --repo retrokiller543/overseerd --title 'Widen plugin installation ordinals' \
         --body 'Plugin installation provenance exceeded u32::MAX; migrate the ordinal to u64 \
         or usize.'`"
    )
}
