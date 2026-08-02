use std::collections::{BTreeMap, BTreeSet};
use std::io;

use cargo_overseerd::GraphView;
use overseerd_tooling_schema::renderer::RendererPresentation;
use overseerd_tooling_schema::{Relationship, Resource};

use crate::cli::GraphFormat;

use super::detail::{terminal_text, write_diagnostics, write_heading};
use super::name::{relationship_kind_name, resource_kind_name};

/// Writes one graph in its requested deterministic representation.
pub(crate) fn write_graph(
    view: &GraphView,
    format: GraphFormat,
    color: bool,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    write_graph_with_presentation(view, format, None, color, output)
}

/// Writes one graph with optional validated text-only node labels.
pub(crate) fn write_graph_with_presentation(
    view: &GraphView,
    format: GraphFormat,
    presentation: Option<&RendererPresentation>,
    color: bool,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    match format {
        GraphFormat::Text => write_text(view, presentation, color, output),
        GraphFormat::Mermaid => write_mermaid(view, output),
        GraphFormat::Dot => write_dot(view, output),
        GraphFormat::Json => {
            let json = view.to_canonical_json().map_err(io::Error::other)?;

            writeln!(output, "{json}")
        }
    }
}

fn write_text(
    view: &GraphView,
    presentation: Option<&RendererPresentation>,
    color: bool,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    let diagnostics = diagnostic_resources(view);
    let nodes = sorted_nodes(view);
    let edges = sorted_edges(view);

    write_heading(output, "Application", color)?;
    writeln!(
        output,
        "  name: {}",
        terminal_text(&view.source.identity.application)
    )?;
    writeln!(
        output,
        "  protocol: {}",
        optional_text(&view.source.protocol)
    )?;

    if !view.complete {
        writeln!(output, "  graph: diagnostic-only/incomplete")?;
        writeln!(
            output,
            "  failure phase: {}",
            optional_text(&view.failure_phase)
        )?;
    }

    writeln!(output, "  family: {:?}", view.family)?;
    writeln!(output, "  direction: {:?}", view.direction)?;

    write_heading(output, "Nodes", color)?;

    if nodes.is_empty() {
        writeln!(output, "  none")?;
    }

    for node in nodes {
        let marker = if diagnostics.contains(node.id.as_str()) {
            colored_marker(color)
        } else {
            " "
        };

        let name = presentation
            .and_then(|presentation| presentation.resource(&node.id))
            .and_then(|presentation| presentation.label.as_deref())
            .unwrap_or(&node.name);

        writeln!(
            output,
            "  {marker} {:<14} {} ({})",
            resource_kind_name(&node.kind),
            terminal_text(name),
            terminal_text(&node.id)
        )?;
    }

    write_heading(output, "Edges", color)?;

    if edges.is_empty() {
        writeln!(output, "  none")?;
    }

    for edge in edges {
        writeln!(
            output,
            "  {:<14} {} -> {}{}",
            relationship_kind_name(&edge.kind),
            terminal_text(&edge.from),
            terminal_text(&edge.to),
            labels_suffix(&edge.labels)
        )?;
    }

    if !view.diagnostics.is_empty() {
        write_heading(output, "Diagnostics", color)?;
        write_diagnostics(&view.diagnostics, "  ", output)?;
    }

    output.flush()
}

fn write_mermaid(view: &GraphView, output: &mut dyn io::Write) -> io::Result<()> {
    let nodes = sorted_nodes(view);
    let edges = sorted_edges(view);
    let ordinals = node_ordinals(&nodes);

    writeln!(output, "flowchart LR")?;

    for (ordinal, node) in nodes.iter().enumerate() {
        let label = node_label(view, node);

        writeln!(output, "  n{ordinal}[\"{}\"]", escape_mermaid(&label))?;
    }

    for edge in edges {
        let from = ordinals[edge.from.as_str()];
        let to = ordinals[edge.to.as_str()];
        let label = edge_label(edge);

        writeln!(
            output,
            "  n{from} -->|\"{}\"| n{to}",
            escape_mermaid(&label)
        )?;
    }

    output.flush()
}

fn write_dot(view: &GraphView, output: &mut dyn io::Write) -> io::Result<()> {
    let nodes = sorted_nodes(view);
    let edges = sorted_edges(view);
    let ordinals = node_ordinals(&nodes);

    writeln!(output, "digraph overseerd {{")?;

    for (ordinal, node) in nodes.iter().enumerate() {
        let label = node_label(view, node);

        writeln!(output, "  n{ordinal} [label=\"{}\"];", escape_dot(&label))?;
    }

    for edge in edges {
        let from = ordinals[edge.from.as_str()];
        let to = ordinals[edge.to.as_str()];

        writeln!(
            output,
            "  n{from} -> n{to} [label=\"{}\"];",
            escape_dot(&edge_label(edge))
        )?;
    }

    writeln!(output, "}}")?;
    output.flush()
}

fn node_ordinals<'a>(nodes: &[&'a Resource]) -> BTreeMap<&'a str, usize> {
    nodes
        .iter()
        .enumerate()
        .map(|(ordinal, node)| (node.id.as_str(), ordinal))
        .collect()
}

fn node_label(view: &GraphView, node: &Resource) -> String {
    let status = if view.complete {
        String::new()
    } else {
        String::from("\ndiagnostic-only/incomplete")
    };

    format!(
        "{}\n{}\n{}{}",
        resource_kind_name(&node.kind),
        node.name,
        node.id,
        status
    )
}

fn optional_text(value: &Option<String>) -> String {
    value
        .as_deref()
        .map(terminal_text)
        .unwrap_or_else(|| String::from("unavailable"))
}

fn sorted_nodes(view: &GraphView) -> Vec<&Resource> {
    let mut nodes = view.nodes.iter().collect::<Vec<_>>();

    nodes.sort_by(|left, right| left.id.cmp(&right.id));

    nodes
}

fn sorted_edges(view: &GraphView) -> Vec<&Relationship> {
    let mut edges = view.edges.iter().collect::<Vec<_>>();

    edges.sort_by(|left, right| {
        (&left.from, &left.kind, &left.to, &left.labels).cmp(&(
            &right.from,
            &right.kind,
            &right.to,
            &right.labels,
        ))
    });

    edges
}

fn diagnostic_resources(view: &GraphView) -> BTreeSet<&str> {
    view.diagnostics
        .iter()
        .flat_map(|diagnostic| diagnostic.resources.iter().map(String::as_str))
        .collect()
}

fn edge_label(edge: &Relationship) -> String {
    let suffix = labels_suffix(&edge.labels);

    format!("{}{}", relationship_kind_name(&edge.kind), suffix)
}

fn labels_suffix(labels: &BTreeMap<String, String>) -> String {
    if labels.is_empty() {
        return String::new();
    }

    let labels = labels
        .iter()
        .map(|(name, value)| format!("{}={}", terminal_text(name), terminal_text(value)))
        .collect::<Vec<_>>()
        .join(", ");

    format!(" [{labels}]")
}

fn colored_marker(color: bool) -> &'static str {
    if color { "\u{1b}[1;31m!\u{1b}[0m" } else { "!" }
}

fn escape_mermaid(value: &str) -> String {
    escape_quoted(value, "#34;", "#92;")
}

fn escape_dot(value: &str) -> String {
    escape_quoted(value, "\\\"", "\\\\")
}

fn escape_quoted(value: &str, quote: &str, slash: &str) -> String {
    let mut escaped = String::new();

    for character in value.chars() {
        match character {
            '"' => escaped.push_str(quote),
            '\\' => escaped.push_str(slash),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write as _;

                write!(escaped, "\\u{:04x}", character as u32)
                    .expect("writing to a string cannot fail");
            }
            character => escaped.push(character),
        }
    }

    escaped
}

#[cfg(test)]
mod tests;
