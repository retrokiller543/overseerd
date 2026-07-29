use overseerd_tooling_schema::{Diagnostic, DiagnosticSeverity, ProbeFailure};

use super::{contribution_id, contributor_id};
use crate::{ContributionProvenance, InstallationOrigin, InstallationProvenance};

mod di;

pub(in crate::tooling) use di::di_failure;

pub(super) fn app_diagnostics(error: &crate::Error, phase: Option<String>) -> Option<ProbeFailure> {
    let diagnostics = match error {
        crate::Error::Composition(error) => error
            .as_slice()
            .iter()
            .map(composition_diagnostic)
            .collect(),
        crate::Error::PluginPlan(error) => vec![plugin_plan_diagnostic(error)],
        crate::Error::ScopeTopology(error) => vec![scope_topology_diagnostic(error)],
        _ => return None,
    };

    Some(ProbeFailure { phase, diagnostics })
}

fn composition_diagnostic(error: &crate::CompositionDiagnostic) -> Diagnostic {
    use crate::CompositionDiagnostic;

    let resources = match error {
        CompositionDiagnostic::ReservedNamespace { plugin, provenance } => {
            vec![plugin_resource(*plugin), installation_resource(*provenance)]
        }
        CompositionDiagnostic::ReservedSlotNamespace { slot, provenance }
        | CompositionDiagnostic::ReplacementTargetMissing { slot, provenance }
        | CompositionDiagnostic::ReplacementForbidden {
            slot, provenance, ..
        }
        | CompositionDiagnostic::SuppressionTargetMissing { slot, provenance }
        | CompositionDiagnostic::SuppressionForbidden {
            slot, provenance, ..
        }
        | CompositionDiagnostic::LateSlotMutation { slot, provenance } => vec![
            plugin_slot_resource(*slot),
            installation_resource(*provenance),
        ],
        CompositionDiagnostic::ProtocolOriginMismatch {
            selected,
            declared,
            provenance,
        } => vec![
            protocol_resource(*selected),
            protocol_resource(*declared),
            installation_resource(*provenance),
        ],
        CompositionDiagnostic::DuplicatePlugin { id, first, second } => vec![
            plugin_resource(*id),
            installation_resource(*first),
            installation_resource(*second),
        ],
        CompositionDiagnostic::DuplicateSlot {
            slot,
            first_plugin,
            first,
            second_plugin,
            second,
        } => vec![
            plugin_slot_resource(*slot),
            plugin_resource(*first_plugin),
            installation_resource(*first),
            plugin_resource(*second_plugin),
            installation_resource(*second),
        ],
        CompositionDiagnostic::MissingDependency {
            plugin,
            provenance,
            target,
        }
        | CompositionDiagnostic::SelfDependency {
            plugin,
            provenance,
            target,
        }
        | CompositionDiagnostic::SelfConflict {
            plugin,
            provenance,
            target,
        }
        | CompositionDiagnostic::SelfOrdering {
            plugin,
            provenance,
            target,
            ..
        } => vec![
            plugin_resource(*plugin),
            installation_resource(*provenance),
            relation_target_resource(*target),
        ],
        CompositionDiagnostic::Conflict {
            left,
            left_provenance,
            right,
            right_provenance,
        } => vec![
            plugin_resource(*left),
            installation_resource(*left_provenance),
            plugin_resource(*right),
            installation_resource(*right_provenance),
        ],
        CompositionDiagnostic::MultipleReplacements {
            slot,
            first,
            second,
        }
        | CompositionDiagnostic::MultipleSuppressions {
            slot,
            first,
            second,
        }
        | CompositionDiagnostic::ConflictingSlotDirectives {
            slot,
            first,
            second,
        } => vec![
            plugin_slot_resource(*slot),
            installation_resource(*first),
            installation_resource(*second),
        ],
        CompositionDiagnostic::ReplacementDeclaresSlot {
            target,
            declared,
            plugin,
            provenance,
        } => vec![
            plugin_slot_resource(*target),
            plugin_slot_resource(*declared),
            plugin_resource(*plugin),
            installation_resource(*provenance),
        ],
        CompositionDiagnostic::UnexpectedPhase { provenance, .. } => {
            vec![installation_resource(*provenance)]
        }
        CompositionDiagnostic::LateOrderingBeforeEarly {
            plugin,
            target,
            provenance,
        }
        | CompositionDiagnostic::EarlyOrderingAfterLate {
            plugin,
            target,
            provenance,
        } => vec![
            plugin_resource(*plugin),
            plugin_resource(*target),
            installation_resource(*provenance),
        ],
        CompositionDiagnostic::Cycle { members, .. } => members
            .iter()
            .flat_map(|member| {
                [
                    plugin_resource(member.plugin()),
                    installation_resource(member.provenance()),
                ]
            })
            .collect(),
    };

    framework_diagnostic(
        "overseerd/tooling-plugin-composition",
        error.to_string(),
        resources,
        "Correct the conflicting plugin declarations or dependencies.",
    )
}

fn plugin_plan_diagnostic(error: &crate::PluginPlanError) -> Diagnostic {
    use crate::PluginPlanError;

    let resources = match error {
        #[cfg(feature = "cli")]
        PluginPlanError::DuplicateCliProvider { provenance }
        | PluginPlanError::ReservedCliProviderNamespace { provenance } => {
            contribution_resources(*provenance)
        }
        PluginPlanError::DuplicateContribution { provenance }
        | PluginPlanError::ReservedContributionNamespace { provenance } => {
            contribution_resources(*provenance)
        }
        PluginPlanError::Tooling(_) => Vec::new(),
        PluginPlanError::MissingInstallation { plugin, provenance } => {
            vec![plugin_resource(*plugin), installation_resource(*provenance)]
        }
    };

    framework_diagnostic(
        "overseerd/tooling-plugin-plan",
        "Plugin contributions could not be lowered into the application plan.",
        resources,
        "Correct duplicate or reserved plugin contribution identities.",
    )
}

fn scope_topology_diagnostic(error: &crate::ScopeTopologyError) -> Diagnostic {
    let resources = scope_topology_resources(error);

    framework_diagnostic(
        "overseerd/tooling-scope-topology",
        error.to_string(),
        resources,
        "Correct duplicate, missing, cyclic, or invalid scope parent declarations.",
    )
}

pub(super) fn scope_topology_resources(error: &crate::ScopeTopologyError) -> Vec<String> {
    use crate::ScopeTopologyError;

    match error {
        ScopeTopologyError::DuplicateId { id }
        | ScopeTopologyError::ReservedId { id }
        | ScopeTopologyError::SelfParent { id } => vec![scope_resource(*id)],
        ScopeTopologyError::MissingParent { id, parent } => {
            vec![scope_resource(*id), scope_resource(*parent)]
        }
        ScopeTopologyError::Cycle { members } => {
            members.iter().map(|id| scope_resource(*id)).collect()
        }
        ScopeTopologyError::InvalidParentRank { child, parent, .. } => {
            vec![scope_resource(*child), scope_resource(*parent)]
        }
    }
}

#[cfg(feature = "cli")]
pub(super) fn cli_definition_diagnostic(error: &crate::CliDefinitionError) -> Diagnostic {
    let mut resources = vec![
        format!("cli-command:{}", error.command()),
        format!("cli-definition:{}:{}", error.kind(), error.value()),
    ];

    resources.extend(cli_definition_source_resources(error.first()));
    resources.extend(cli_definition_source_resources(error.second()));

    framework_diagnostic(
        "overseerd/tooling-cli-definition",
        "The generated command-line definition is structurally invalid.",
        resources,
        "Rename or remove the conflicting command-line declaration.",
    )
}

#[cfg(feature = "cli")]
fn cli_definition_source_resources(source: crate::CliDefinitionSource) -> Vec<String> {
    match source {
        crate::CliDefinitionSource::Framework => vec![String::from("framework")],
        crate::CliDefinitionSource::Application => vec![String::from("application")],
        crate::CliDefinitionSource::Plugin(provenance) => contribution_resources(provenance),
    }
}

fn framework_diagnostic(
    code: &str,
    message: impl Into<String>,
    resources: Vec<String>,
    fix: &str,
) -> Diagnostic {
    Diagnostic {
        code: code.to_string(),
        severity: DiagnosticSeverity::Error,
        message: message.into(),
        resources,
        sources: Vec::new(),
        fix: Some(fix.to_string()),
    }
}

fn contribution_resources(provenance: ContributionProvenance) -> Vec<String> {
    vec![
        contributor_id(provenance.contributor()),
        contribution_id(provenance),
    ]
}

fn relation_target_resource(target: crate::RelationTarget) -> String {
    match target {
        crate::RelationTarget::Plugin(plugin) => plugin_resource(plugin),
        crate::RelationTarget::Slot(slot) => plugin_slot_resource(slot),
    }
}

fn installation_resource(provenance: InstallationProvenance) -> String {
    let origin = match provenance.origin() {
        InstallationOrigin::ProtocolMandatory(protocol) => {
            format!("protocol-mandatory:{}", protocol.as_str())
        }
        InstallationOrigin::ProtocolDefault(protocol) => {
            format!("protocol-default:{}", protocol.as_str())
        }
        InstallationOrigin::ApplicationDeclaration => String::from("application-declaration"),
        InstallationOrigin::ApplicationConfiguration => String::from("application-configuration"),
    };

    format!("plugin-installation:{origin}:{}", provenance.ordinal())
}

fn plugin_resource(plugin: crate::PluginId) -> String {
    format!("plugin:{}", plugin.as_str())
}

fn plugin_slot_resource(slot: crate::PluginSlotId) -> String {
    format!("plugin-slot:{}", slot.as_str())
}

fn protocol_resource(protocol: crate::ProtocolId) -> String {
    format!("protocol:{}", protocol.as_str())
}

fn scope_resource(scope: overseerd_core::ScopeId) -> String {
    format!("scope:{scope}")
}
