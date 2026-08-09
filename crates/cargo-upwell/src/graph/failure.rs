use std::collections::{BTreeMap, BTreeSet};

use semver::Version;
use upwell_tooling_schema::{Diagnostic, DocumentIdentity, ProbeFailure, Resource, ResourceKind};

use crate::TOOLING_SCHEMA_VERSION;

use super::error::{GraphQueryError, GraphSelectorKind};
use super::model::{GraphQuery, GraphSource, GraphView, diagnostic_cmp};
use super::relation::is_contributor;

const DIAGNOSTIC_STATUS_LABEL: &str = "diagnostic-only";
const INCOMPLETE_LABEL: &str = "incomplete";

/// Builds an incomplete graph from the authoritative diagnostics of a failed preparation probe.
pub fn query_failure_graph(
    envelope_schema: Version,
    identity: &DocumentIdentity,
    failure: &ProbeFailure,
    query: &GraphQuery,
) -> Result<GraphView, GraphQueryError> {
    let mut diagnostics = canonical_diagnostics(&failure.diagnostics);
    let mut nodes = diagnostic_nodes(&mut diagnostics, &failure.resource_kinds);
    let selected = resolve_query_roots(&nodes, query)?;

    if query.has_selectors() {
        diagnostics.retain(|diagnostic| {
            diagnostic
                .resources
                .iter()
                .any(|resource| selected.contains(resource))
        });
        let mut visible = selected.clone();

        visible.extend(
            diagnostics
                .iter()
                .flat_map(|diagnostic| diagnostic.resources.iter().cloned()),
        );
        nodes.retain(|node| visible.contains(&node.id));
    }

    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    diagnostics.sort_by(diagnostic_cmp);

    Ok(GraphView {
        schema: TOOLING_SCHEMA_VERSION,
        source: GraphSource {
            schema: envelope_schema,
            framework_version: None,
            identity: identity.clone(),
            protocol: None,
        },
        complete: false,
        failure_phase: failure.phase.clone(),
        family: query.family,
        direction: query.direction,
        roots: selected.into_iter().collect(),
        nodes,
        edges: Vec::new(),
        diagnostics,
        cli_providers: Vec::new(),
    })
}

fn canonical_diagnostics(diagnostics: &[Diagnostic]) -> Vec<Diagnostic> {
    let mut diagnostics = diagnostics.to_vec();

    for diagnostic in &mut diagnostics {
        diagnostic.resources.sort();
        diagnostic.resources.dedup();
        diagnostic.sources.sort();
        diagnostic.sources.dedup();
    }

    diagnostics.sort_by(diagnostic_cmp);

    diagnostics
}

fn diagnostic_nodes(
    diagnostics: &mut [Diagnostic],
    resource_kinds: &BTreeMap<String, ResourceKind>,
) -> Vec<Resource> {
    let mut identities = diagnostics
        .iter()
        .flat_map(|diagnostic| diagnostic.resources.iter().cloned())
        .collect::<BTreeSet<_>>();
    let mut unattributed = 0_usize;

    for diagnostic in diagnostics {
        if !diagnostic.resources.is_empty() {
            continue;
        }

        let id = loop {
            let candidate = format!("diagnostic:unattributed:{unattributed:04}");

            unattributed += 1;

            if identities.insert(candidate.clone()) {
                break candidate;
            }
        };

        diagnostic.resources.push(id);
    }

    identities
        .iter()
        .map(|id| {
            let kind = resource_kinds
                .get(id)
                .cloned()
                .unwrap_or_else(|| upwell_tooling_schema::diagnostic_resource_kind(id));

            diagnostic_node(id.clone(), kind)
        })
        .collect()
}

fn diagnostic_node(id: String, kind: ResourceKind) -> Resource {
    Resource {
        name: id.clone(),
        id,
        kind,
        labels: [
            (
                String::from("graph-status"),
                String::from(DIAGNOSTIC_STATUS_LABEL),
            ),
            (
                String::from("graph-completeness"),
                String::from(INCOMPLETE_LABEL),
            ),
        ]
        .into_iter()
        .collect(),
        ..Resource::default()
    }
}

fn resolve_query_roots(
    nodes: &[Resource],
    query: &GraphQuery,
) -> Result<BTreeSet<String>, GraphQueryError> {
    let resources = nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut names = BTreeMap::<&str, Vec<&Resource>>::new();
    let mut roots = BTreeSet::new();

    for node in nodes {
        names.entry(&node.name).or_default().push(node);
    }

    for selector in &query.resources {
        roots.insert(resolve(
            &resources,
            &names,
            selector,
            GraphSelectorKind::Resource,
            |_| true,
        )?);
    }

    for selector in &query.contributors {
        roots.insert(resolve(
            &resources,
            &names,
            selector,
            GraphSelectorKind::Contributor,
            |node| is_contributor(&node.kind),
        )?);
    }

    for selector in &query.plugins {
        roots.insert(resolve(
            &resources,
            &names,
            selector,
            GraphSelectorKind::Plugin,
            |node| node.kind == ResourceKind::Plugin,
        )?);
    }

    Ok(roots)
}

fn resolve(
    resources: &BTreeMap<&str, &Resource>,
    names: &BTreeMap<&str, Vec<&Resource>>,
    selector: &str,
    kind: GraphSelectorKind,
    eligible: impl Fn(&Resource) -> bool,
) -> Result<String, GraphQueryError> {
    if let Some(resource) = resources.get(selector) {
        if eligible(resource) {
            return Ok(resource.id.clone());
        }

        return Err(GraphQueryError::NotFound {
            kind,
            selector: selector.to_string(),
        });
    }

    let candidates = names
        .get(selector)
        .into_iter()
        .flatten()
        .filter(|resource| eligible(resource))
        .map(|resource| resource.id.clone())
        .collect::<Vec<_>>();

    match candidates.as_slice() {
        [] => Err(GraphQueryError::NotFound {
            kind,
            selector: selector.to_string(),
        }),
        [id] => Ok(id.clone()),
        _ => Err(GraphQueryError::Ambiguous {
            kind,
            selector: selector.to_string(),
            candidates,
        }),
    }
}

#[cfg(test)]
mod tests;
