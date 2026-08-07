use std::collections::BTreeSet;

use upwell_tooling_schema::{Diagnostic, ToolingDocument};

use crate::TOOLING_SCHEMA_VERSION;

use super::error::GraphQueryError;
use super::index::GraphIndex;
use super::model::{GraphQuery, GraphView};
use super::relation::relation_matches;

/// Executes a deterministic graph request against a tooling document.
pub fn query_graph(
    document: &ToolingDocument,
    query: &GraphQuery,
) -> Result<GraphView, GraphQueryError> {
    let index = GraphIndex::new(document)?;
    let resolved = index.resolve_query_roots(query)?;
    let selected = if query.has_selectors() {
        let seeds = index.expand_owner_roots(&resolved.roots, &resolved.owner_roots);

        index.traverse(&seeds, query.family, query.direction)
    } else {
        index.resource_ids()
    };
    let nodes = selected
        .iter()
        .map(|id| index.resource(id).clone())
        .collect();
    let edges = index
        .document
        .relationships
        .iter()
        .filter(|edge| relation_matches(&index, edge, query.family))
        .filter(|edge| selected.contains(&edge.from) && selected.contains(&edge.to))
        .cloned()
        .collect();
    let diagnostics = relevant_diagnostics(&index.document.diagnostics, &selected);
    let cli_providers = index.relevant_cli_providers(&selected);

    Ok(GraphView {
        schema: TOOLING_SCHEMA_VERSION,
        source: index.source(),
        complete: true,
        failure_phase: None,
        family: query.family,
        direction: query.direction,
        roots: resolved.roots.into_iter().collect(),
        nodes,
        edges,
        diagnostics,
        cli_providers,
    })
}

pub(super) fn relevant_diagnostics(
    diagnostics: &[Diagnostic],
    selected: &BTreeSet<String>,
) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.resources.is_empty()
                || diagnostic
                    .resources
                    .iter()
                    .any(|resource| selected.contains(resource))
        })
        .cloned()
        .collect()
}
