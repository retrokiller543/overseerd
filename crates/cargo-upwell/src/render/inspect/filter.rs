use std::collections::{BTreeMap, BTreeSet};

use upwell_tooling_schema::{Resource, ResourceKind, ToolingDocument};

use crate::cli::{InspectFilters, InspectResourceKind};

pub(super) fn selected_resources<'a>(
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
