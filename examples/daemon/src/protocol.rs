//! Homeledger's RPC protocol composition and audit-policy defaults.

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use overseerd::daemon::{PreparedRpc, Rpc, RpcRuntime};
use overseerd::{
    AppRegistry, AppRuntime, ContributionId, Plugin, PluginCliRegistrar, PluginContributions,
    PluginId, PluginSlotId, PreBuildContext, PreparedProtocol, ProtocolDefinition,
    ProtocolPluginRegistrar, ValidationContext, config, namespaced_id,
};
use serde::Deserialize;

/// Replaceable protocol slot for Homeledger's effective audit policy.
pub const AUDIT_POLICY_SLOT: PluginSlotId = namespaced_id!(PluginSlotId, "homeledger/audit-policy");

/// Optional protocol slot for exporting audit records outside Homeledger.
pub const AUDIT_EXPORT_SLOT: PluginSlotId = namespaced_id!(PluginSlotId, "homeledger/audit-export");

#[cfg(test)]
static PROTOCOL_BUILDS: AtomicUsize = AtomicUsize::new(0);

/// Audit requirements injected into the transaction service.
#[config]
#[derive(Deserialize)]
pub struct AuditPolicyConfig {
    pub retention_days: u16,
    pub require_review_ticket: bool,
}

/// Optional external audit-export settings supplied by the protocol default.
#[config]
#[derive(Deserialize)]
pub struct AuditExportConfig {
    #[serde(rename = "destination")]
    pub _destination: String,
}

/// Global compliance context contributed by the selected audit-policy plugin.
#[derive(clap::Args)]
pub struct ComplianceAuditArgs {
    /// Review ticket attached to compliance-sensitive Homeledger operations.
    #[arg(long, global = true, default_value = "local-review")]
    pub(crate) audit_review_ticket: String,
}

/// Homeledger's household-oriented default audit policy.
#[derive(Default)]
pub struct HouseholdAuditPolicyPlugin;

impl Plugin for HouseholdAuditPolicyPlugin {
    const ID: PluginId = namespaced_id!(PluginId, "homeledger/audit-policy-household");

    fn contribute(self, contributions: &mut PluginContributions) {
        contributions.config::<AuditPolicyConfig>(
            namespaced_id!(ContributionId, "homeledger/audit-policy-config"),
            "homeledger.audit",
        );
    }
}

/// Compliance audit policy selected by the shipped Homeledger application.
#[derive(Default)]
pub struct ComplianceAuditPolicyPlugin;

impl Plugin for ComplianceAuditPolicyPlugin {
    const ID: PluginId = namespaced_id!(PluginId, "homeledger/audit-policy-compliance");

    fn contribute(self, contributions: &mut PluginContributions) {
        contributions.config::<AuditPolicyConfig>(
            namespaced_id!(ContributionId, "homeledger/audit-policy-config"),
            "homeledger.audit",
        );
    }

    fn cli(&self, cli: &mut PluginCliRegistrar) {
        cli.args::<ComplianceAuditArgs>(namespaced_id!(
            ContributionId,
            "homeledger/compliance-audit-args"
        ));
    }
}

/// Optional external audit exporter enabled by the protocol unless an application suppresses it.
#[derive(Default)]
pub struct AuditExportPlugin;

impl Plugin for AuditExportPlugin {
    const ID: PluginId = namespaced_id!(PluginId, "homeledger/audit-export");

    fn contribute(self, contributions: &mut PluginContributions) {
        contributions.config::<AuditExportConfig>(
            namespaced_id!(ContributionId, "homeledger/audit-export-config"),
            "homeledger.audit_export",
        );
    }
}

/// RPC protocol definition with Homeledger-owned audit extension slots.
#[derive(Default)]
pub struct HomeledgerRpc {
    rpc: Rpc,
}

impl ProtocolDefinition for HomeledgerRpc {
    type Prepared = PreparedHomeledgerRpc;
    type Error = overseerd::daemon::Error;

    const ID: overseerd::ProtocolId = namespaced_id!(overseerd::ProtocolId, "homeledger/rpc");
    const SCOPE_TOPOLOGY: overseerd::ScopeTopology = Rpc::SCOPE_TOPOLOGY;

    fn register(&self, registry: &mut AppRegistry) {
        self.rpc.register(registry);
    }

    fn prepare(self, context: &ValidationContext<'_>) -> Result<Self::Prepared, Self::Error> {
        let rpc = self.rpc.prepare(context)?;

        Ok(PreparedHomeledgerRpc { rpc })
    }

    fn auto_discover(&mut self) {
        self.rpc.auto_discover();
    }

    fn register_plugins(plugins: &mut ProtocolPluginRegistrar) {
        Rpc::register_plugins(plugins);
        plugins.replaceable_default(AUDIT_POLICY_SLOT, HouseholdAuditPolicyPlugin);
        plugins.optional_default(AUDIT_EXPORT_SLOT, AuditExportPlugin);
    }

    fn pre_build(&mut self, context: &mut PreBuildContext<'_>) -> Result<(), Self::Error> {
        self.rpc.pre_build(context)
    }
}

/// Prepared Homeledger protocol retaining the validated RPC service plan.
pub struct PreparedHomeledgerRpc {
    rpc: PreparedRpc,
}

impl PreparedProtocol for PreparedHomeledgerRpc {
    type Runtime = RpcRuntime;
    type Error = overseerd::daemon::Error;

    fn build(self, runtime: &AppRuntime) -> Result<Self::Runtime, Self::Error> {
        #[cfg(test)]
        PROTOCOL_BUILDS.fetch_add(1, Ordering::SeqCst);

        self.rpc.build(runtime)
    }

    fn tooling(&self, contributions: &mut overseerd::tooling::ToolingContributions) {
        self.rpc.tooling(contributions);
    }
}

#[cfg(test)]
pub(crate) fn reset_protocol_builds() {
    PROTOCOL_BUILDS.store(0, Ordering::SeqCst);
}

#[cfg(test)]
pub(crate) fn protocol_builds() -> usize {
    PROTOCOL_BUILDS.load(Ordering::SeqCst)
}
