use std::collections::{BTreeMap, BTreeSet};

use upwell_tooling_schema::{CliCommand, CliOwner, Resource, ResourceKind, ToolingDocument};

use crate::cli::{InspectFilters, InspectResourceKind};

pub(crate) fn selected_resources<'a>(
    document: &'a ToolingDocument,
    filters: &InspectFilters,
) -> Vec<&'a Resource> {
    let resource_by_id = document
        .resources
        .iter()
        .map(|resource| (resource.id.as_str(), resource))
        .collect::<BTreeMap<_, _>>();
    let scopes = scope_filter_values(document, &filters.scopes);

    document
        .resources
        .iter()
        .filter(|resource| matches_resource(resource, &resource_by_id, &scopes, filters))
        .collect()
}

pub(crate) fn project_inspection(
    document: &ToolingDocument,
    filters: &InspectFilters,
) -> std::io::Result<ToolingDocument> {
    if filters.is_empty() {
        return Ok(document.clone());
    }

    let selected = selected_resources(document, filters)
        .into_iter()
        .map(|resource| resource.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut projection = document.clone();

    projection
        .resources
        .retain(|resource| selected.contains(resource.id.as_str()));
    for resource in &mut projection.resources {
        if resource
            .provenance
            .as_ref()
            .and_then(|provenance| provenance.owner.as_deref())
            .is_some_and(|owner| !selected.contains(owner))
            && let Some(provenance) = &mut resource.provenance
        {
            provenance.owner = None;
        }
    }
    projection.relationships.retain(|relationship| {
        selected.contains(relationship.from.as_str()) || selected.contains(relationship.to.as_str())
    });
    let connected = projection
        .relationships
        .iter()
        .flat_map(|relationship| [&relationship.from, &relationship.to])
        .collect::<BTreeSet<_>>();
    for resource in document.resources.iter().filter(|resource| {
        connected.contains(&resource.id) && !selected.contains(resource.id.as_str())
    }) {
        projection.resources.push(resource.clone());
    }
    projection.diagnostics.retain_mut(|diagnostic| {
        if diagnostic.resources.is_empty() {
            return true;
        }

        diagnostic
            .resources
            .retain(|resource| selected.contains(resource.as_str()));
        !diagnostic.resources.is_empty()
    });
    projection
        .facets
        .retain(|namespace, _| filters.facets.is_empty() || filters.facets.contains(namespace));
    let cli_resources = project_cli(document, &mut projection, filters);
    let projected_ids = projection
        .resources
        .iter()
        .map(|resource| resource.id.clone())
        .collect::<BTreeSet<_>>();
    for resource in document.resources.iter().filter(|resource| {
        cli_resources.contains(&resource.id) && !projected_ids.contains(&resource.id)
    }) {
        projection.resources.push(resource.clone());
    }
    projection.canonicalize();
    projection.validate().map_err(std::io::Error::other)?;

    Ok(projection)
}

fn project_cli(
    document: &ToolingDocument,
    projection: &mut ToolingDocument,
    filters: &InspectFilters,
) -> BTreeSet<String> {
    if !filters.kinds.is_empty() || !filters.facets.is_empty() || !filters.scopes.is_empty() {
        projection.cli = None;

        return BTreeSet::new();
    }
    let Some(cli) = &mut projection.cli else {
        return BTreeSet::new();
    };
    let provider_ids = cli
        .providers
        .iter()
        .filter(|provider| super::matches_cli_provider(document, provider, filters))
        .map(|provider| provider.id.clone())
        .collect::<BTreeSet<_>>();

    cli.default_command = None;
    cli.providers
        .retain(|provider| provider_ids.contains(&provider.id));
    filter_cli_command(&mut cli.root, &provider_ids, true);

    cli.providers
        .iter()
        .flat_map(|provider| {
            [
                provider.contributor.clone(),
                format!(
                    "contribution:{}:{}",
                    provider.contributor, provider.contribution
                ),
            ]
        })
        .collect()
}

fn filter_cli_command(command: &mut CliCommand, providers: &BTreeSet<String>, root: bool) -> bool {
    command
        .arguments
        .retain(|argument| owner_uses_provider(&argument.owner, providers));
    command
        .commands
        .retain_mut(|command| filter_cli_command(command, providers, false));

    root || owner_uses_provider(&command.owner, providers)
        || !command.arguments.is_empty()
        || !command.commands.is_empty()
}

fn owner_uses_provider(owner: &CliOwner, providers: &BTreeSet<String>) -> bool {
    matches!(owner, CliOwner::Plugin { provider } if providers.contains(provider))
}

fn matches_resource(
    resource: &Resource,
    resource_by_id: &BTreeMap<&str, &Resource>,
    scopes: &BTreeSet<&str>,
    filters: &InspectFilters,
) -> bool {
    matches_kind(resource, &filters.kinds)
        && matches_value(&filters.resources, [&resource.id, &resource.name])
        && matches_owner(resource, resource_by_id, &filters.contributors)
        && matches_plugin(resource, resource_by_id, &filters.plugins)
        && matches_facet(resource, &filters.facets)
        && matches_scope(resource, scopes)
}

fn matches_kind(resource: &Resource, filters: &[InspectResourceKind]) -> bool {
    filters.is_empty()
        || filters
            .iter()
            .any(|filter| resource_kind_matches(&resource.kind, *filter))
}

fn matches_owner(
    resource: &Resource,
    resource_by_id: &BTreeMap<&str, &Resource>,
    filters: &[String],
) -> bool {
    if filters.is_empty() {
        return true;
    }

    let Some(owner) = resource
        .provenance
        .as_ref()
        .and_then(|provenance| provenance.owner.as_deref())
    else {
        return false;
    };
    let owner_name = resource_by_id
        .get(owner)
        .map(|resource| resource.name.as_str());

    filters
        .iter()
        .any(|filter| filter == owner || owner_name == Some(filter.as_str()))
}

fn matches_plugin(
    resource: &Resource,
    resource_by_id: &BTreeMap<&str, &Resource>,
    filters: &[String],
) -> bool {
    if filters.is_empty() {
        return true;
    }

    if matches_value(filters, [&resource.id, &resource.name])
        && matches!(resource.kind, ResourceKind::Plugin)
    {
        return true;
    }

    let Some(owner) = resource
        .provenance
        .as_ref()
        .and_then(|provenance| provenance.owner.as_deref())
    else {
        return false;
    };
    let Some(owner_resource) = resource_by_id.get(owner) else {
        return false;
    };

    matches!(owner_resource.kind, ResourceKind::Plugin)
        && matches_value(filters, [&owner_resource.id, &owner_resource.name])
}

fn matches_facet(resource: &Resource, filters: &[String]) -> bool {
    filters.is_empty()
        || filters
            .iter()
            .any(|filter| resource.facets.contains_key(filter))
}

fn matches_scope(resource: &Resource, filters: &BTreeSet<&str>) -> bool {
    if filters.is_empty() {
        return true;
    }

    let scope = resource.labels.get("scope");
    let scope_id = resource.labels.get("scope-id");

    filters.iter().any(|filter| {
        (matches!(resource.kind, ResourceKind::Scope)
            && (*filter == resource.id || *filter == resource.name))
            || scope.is_some_and(|scope| scope == *filter)
            || scope_id.is_some_and(|scope| scope == *filter)
    })
}

fn scope_filter_values<'a>(
    document: &'a ToolingDocument,
    filters: &'a [String],
) -> BTreeSet<&'a str> {
    let mut values = filters.iter().map(String::as_str).collect::<BTreeSet<_>>();

    for resource in document
        .resources
        .iter()
        .filter(|resource| matches!(resource.kind, ResourceKind::Scope))
    {
        let scope_id = resource.labels.get("scope-id").map(String::as_str);
        let selected = filters.iter().any(|filter| {
            filter == &resource.id || filter == &resource.name || scope_id == Some(filter.as_str())
        });

        if selected {
            values.insert(resource.id.as_str());
            values.insert(resource.name.as_str());

            if let Some(scope_id) = scope_id {
                values.insert(scope_id);
            }
        }
    }

    values
}

fn matches_value<'a>(filters: &[String], values: impl IntoIterator<Item = &'a String>) -> bool {
    if filters.is_empty() {
        return true;
    }

    let values = values.into_iter().collect::<Vec<_>>();

    filters.iter().any(|filter| values.contains(&filter))
}

fn resource_kind_matches(kind: &ResourceKind, filter: InspectResourceKind) -> bool {
    matches!(
        (kind, filter),
        (ResourceKind::Application, InspectResourceKind::Application)
            | (ResourceKind::Protocol, InspectResourceKind::Protocol)
            | (ResourceKind::Plugin, InspectResourceKind::Plugin)
            | (ResourceKind::Component, InspectResourceKind::Component)
            | (ResourceKind::Provider, InspectResourceKind::Provider)
            | (
                ResourceKind::ConfigBinding,
                InspectResourceKind::ConfigBinding
            )
            | (ResourceKind::Hook, InspectResourceKind::Hook)
            | (ResourceKind::Lifecycle, InspectResourceKind::Lifecycle)
            | (ResourceKind::Scope, InspectResourceKind::Scope)
            | (ResourceKind::Type, InspectResourceKind::Type)
            | (
                ResourceKind::Contribution,
                InspectResourceKind::Contribution
            )
            | (ResourceKind::Contributor, InspectResourceKind::Contributor)
            | (ResourceKind::PluginSlot, InspectResourceKind::PluginSlot)
    )
}
