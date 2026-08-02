use std::io;

use cargo_overseerd::ResourceExplanation;
use overseerd_tooling_schema::renderer::RendererPresentation;

use crate::cli::ExplainFormat;

use super::detail::{source_location, terminal_text, write_diagnostics, write_heading};
use super::name::{
    cli_owner_name, cli_provider_kind_name, relationship_kind_name, resource_kind_name,
};

/// Writes one resource explanation in its requested deterministic representation.
pub(crate) fn write_explanation(
    explanation: &ResourceExplanation,
    format: ExplainFormat,
    color: bool,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    write_explanation_with_presentation(explanation, format, None, color, output)
}

/// Writes one explanation with optional validated owner-specific detail fields.
pub(crate) fn write_explanation_with_presentation(
    explanation: &ResourceExplanation,
    format: ExplainFormat,
    presentation: Option<&RendererPresentation>,
    color: bool,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    match format {
        ExplainFormat::Text => write_text(explanation, presentation, color, output),
        ExplainFormat::Json => {
            let json = explanation.to_canonical_json().map_err(io::Error::other)?;

            writeln!(output, "{json}")
        }
    }
}

fn write_text(
    explanation: &ResourceExplanation,
    presentation: Option<&RendererPresentation>,
    color: bool,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    let resource = &explanation.resource;

    write_heading(output, "Resource", color)?;
    writeln!(output, "  id: {}", terminal_text(&resource.id))?;
    writeln!(output, "  kind: {}", resource_kind_name(&resource.kind))?;
    let rendered = presentation.and_then(|presentation| presentation.resource(&resource.id));
    let name = rendered
        .and_then(|presentation| presentation.label.as_deref())
        .unwrap_or(&resource.name);

    writeln!(output, "  name: {}", terminal_text(name))?;

    write_heading(output, "Provenance", color)?;

    if let Some(provenance) = &resource.provenance {
        write_optional(output, "owner", provenance.owner.as_deref())?;
        write_optional(output, "origin", provenance.origin.as_deref())?;

        if let Some(ordinal) = provenance.ordinal {
            writeln!(output, "  ordinal: {ordinal}")?;
        }

        if let Some(source) = &provenance.source {
            writeln!(output, "  source: {}", source_location(source))?;
        }
    } else {
        writeln!(output, "  none")?;
    }

    write_heading(output, "Labels", color)?;
    write_pairs_or_none(resource.labels.iter(), output)?;

    write_heading(output, "Facets", color)?;

    if resource.facets.is_empty() {
        writeln!(output, "  none")?;
    }

    for (namespace, facet) in &resource.facets {
        writeln!(
            output,
            "  {}@{}: {}",
            terminal_text(namespace),
            facet.schema_version,
            value_summary(&facet.value)
        )?;
    }

    write_relationships(
        "Inbound Relationships",
        &explanation.incoming,
        color,
        output,
    )?;
    write_relationships(
        "Outbound Relationships",
        &explanation.outgoing,
        color,
        output,
    )?;
    write_resolution_edges(explanation, color, output)?;

    write_heading(output, "Diagnostics", color)?;

    if explanation.diagnostics.is_empty() {
        writeln!(output, "  none")?;
    } else {
        write_diagnostics(&explanation.diagnostics, "  ", output)?;
    }

    write_cli(explanation, color, output)?;

    if let Some(rendered) = rendered {
        write_renderer_details(rendered, color, output)?;
    }

    output.flush()
}

fn write_renderer_details(
    presentation: &overseerd_tooling_schema::renderer::ResourcePresentation,
    color: bool,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    write_heading(output, "Renderer Details", color)?;

    if let Some(group) = &presentation.group {
        writeln!(output, "  group: {}", terminal_text(group))?;
    }

    if let Some(summary) = &presentation.summary {
        writeln!(output, "  summary: {}", terminal_text(summary))?;
    }

    for (name, value) in &presentation.details {
        writeln!(
            output,
            "  {}: {}",
            terminal_text(name),
            terminal_text(value)
        )?;
    }

    Ok(())
}

fn write_relationships(
    heading: &str,
    relationships: &[overseerd_tooling_schema::Relationship],
    color: bool,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    write_heading(output, heading, color)?;

    if relationships.is_empty() {
        writeln!(output, "  none")?;
    }

    for relationship in relationships {
        writeln!(
            output,
            "  {} {} -> {}",
            relationship_kind_name(&relationship.kind),
            terminal_text(&relationship.from),
            terminal_text(&relationship.to)
        )?;

        for (name, value) in &relationship.labels {
            writeln!(
                output,
                "    {}: {}",
                terminal_text(name),
                terminal_text(value)
            )?;
        }
    }

    Ok(())
}

fn write_resolution_edges(
    explanation: &ResourceExplanation,
    color: bool,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    let resolution_edges = explanation
        .incoming
        .iter()
        .chain(&explanation.outgoing)
        .filter(|edge| {
            matches!(
                edge.labels.get("role").map(String::as_str),
                Some("resolved-provider" | "resolved-component")
            )
        })
        .collect::<Vec<_>>();

    write_heading(output, "Selected Resolutions", color)?;

    if resolution_edges.is_empty() {
        writeln!(output, "  none")?;
    }

    for edge in resolution_edges {
        writeln!(
            output,
            "  {} {} -> {}",
            terminal_text(&edge.labels["role"]),
            terminal_text(&edge.from),
            terminal_text(&edge.to)
        )?;

        for (name, value) in &edge.labels {
            writeln!(
                output,
                "    {}: {}",
                terminal_text(name),
                terminal_text(value)
            )?;
        }
    }

    Ok(())
}

fn write_cli(
    explanation: &ResourceExplanation,
    color: bool,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    write_heading(output, "CLI Provider Ownership", color)?;

    if explanation.cli_providers.is_empty() {
        writeln!(output, "  none")?;
    }

    for provider in &explanation.cli_providers {
        writeln!(
            output,
            "  {} [{}] contributor={} contribution={}",
            terminal_text(&provider.id),
            cli_provider_kind_name(provider.kind),
            terminal_text(&provider.contributor),
            terminal_text(&provider.contribution)
        )?;
    }

    write_heading(output, "Parser Ownership", color)?;

    if explanation.cli_ownership.commands.is_empty()
        && explanation.cli_ownership.arguments.is_empty()
    {
        writeln!(output, "  none")?;
    }

    for command in &explanation.cli_ownership.commands {
        writeln!(
            output,
            "  command {} [{}]{}",
            terminal_text(&command.path.join(" ")),
            terminal_text(cli_owner_name(&command.owner)),
            command
                .id
                .as_deref()
                .map(|id| format!(" id={}", terminal_text(id)))
                .unwrap_or_default()
        )?;
    }

    for argument in &explanation.cli_ownership.arguments {
        writeln!(
            output,
            "  argument {} {} [{}]",
            terminal_text(&argument.command_path.join(" ")),
            terminal_text(&argument.id),
            terminal_text(cli_owner_name(&argument.owner))
        )?;
    }

    Ok(())
}

fn write_optional(output: &mut dyn io::Write, name: &str, value: Option<&str>) -> io::Result<()> {
    if let Some(value) = value {
        writeln!(
            output,
            "  {}: {}",
            terminal_text(name),
            terminal_text(value)
        )?;
    }

    Ok(())
}

fn write_pairs_or_none<'a>(
    pairs: impl ExactSizeIterator<Item = (&'a String, &'a String)>,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    if pairs.len() == 0 {
        writeln!(output, "  none")?;

        return Ok(());
    }

    for (name, value) in pairs {
        writeln!(
            output,
            "  {}: {}",
            terminal_text(name),
            terminal_text(value)
        )?;
    }

    Ok(())
}

fn value_summary(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::from("null"),
        serde_json::Value::Bool(value) => format!("boolean ({value})"),
        serde_json::Value::Number(value) => format!("number ({value})"),
        serde_json::Value::String(value) => {
            format!("string ({} characters)", value.chars().count())
        }
        serde_json::Value::Array(values) => format!("array ({} items)", values.len()),
        serde_json::Value::Object(values) => format!("object ({} fields)", values.len()),
    }
}

#[cfg(test)]
mod tests;
