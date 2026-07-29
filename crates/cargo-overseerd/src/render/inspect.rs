use std::collections::{BTreeMap, BTreeSet};
use std::io;

use overseerd_tooling_schema::{
    CliArgument, CliCommand, Diagnostic, Facet, Provenance, Resource, ResourceKind, SourceLocation,
    ToolingDocument,
};

use crate::cli::InspectFilters;

mod filter;
mod name;

use filter::selected_resources;
use name::{
    cli_owner_name, cli_provider_kind_matches, cli_provider_kind_name, diagnostic_severity_name,
    relationship_kind_name, resource_kind_name,
};

/// Writes one protocol-neutral human-readable inspection.
pub(crate) fn write_inspection(
    document: &ToolingDocument,
    filters: &InspectFilters,
    color: bool,
    output: &mut impl io::Write,
) -> io::Result<()> {
    let mut document = document.clone();

    document.canonicalize();

    let resources = selected_resources(&document, filters);
    let selected_ids = resources
        .iter()
        .map(|resource| resource.id.as_str())
        .collect::<BTreeSet<_>>();

    write_heading(output, "Application", color)?;
    writeln!(output, "  name: {}", document.identity.application)?;
    writeln!(output, "  protocol: {}", document.protocol)?;
    writeln!(output, "  framework: {}", document.framework_version)?;
    writeln!(output, "  schema: {}", document.schema)?;
    writeln!(
        output,
        "  validation: {}",
        if document.validation.valid {
            "valid"
        } else {
            "invalid"
        }
    )?;
    write_identity(&document, output)?;

    write_heading(output, "Resources", color)?;

    if resources.is_empty() {
        writeln!(output, "  none")?;
    }

    for resource in resources {
        writeln!(
            output,
            "  {} {} ({})",
            resource_kind_name(&resource.kind),
            resource.name,
            resource.id
        )?;
        write_provenance(resource.provenance.as_ref(), output)?;
        write_labels(&resource.labels, output)?;
        write_facets(&resource.facets, output, "    ")?;
    }

    write_heading(output, "Relationships", color)?;

    let mut relationship_count = 0_usize;

    for relationship in &document.relationships {
        let from_selected = selected_ids.contains(relationship.from.as_str());
        let to_selected = selected_ids.contains(relationship.to.as_str());
        let selected = if filters.is_empty() {
            from_selected && to_selected
        } else {
            from_selected || to_selected
        };

        if selected {
            writeln!(
                output,
                "  {} --{}--> {}",
                relationship.from,
                relationship_kind_name(&relationship.kind),
                relationship.to
            )?;
            write_labels(&relationship.labels, output)?;
            relationship_count += 1;
        }
    }

    if relationship_count == 0 {
        writeln!(output, "  none")?;
    }

    write_cli(&document, filters, color, output)?;
    write_diagnostics(&document.diagnostics, color, output)?;

    let document_facets = document
        .facets
        .iter()
        .filter(|(namespace, _)| filters.facets.is_empty() || filters.facets.contains(namespace))
        .map(|(namespace, facet)| (namespace.clone(), facet.clone()))
        .collect::<BTreeMap<_, _>>();

    if !document_facets.is_empty() {
        write_heading(output, "Document Facets", color)?;
        write_facets(&document_facets, output, "  ")?;
    }

    output.flush()
}

fn write_identity(document: &ToolingDocument, output: &mut impl io::Write) -> io::Result<()> {
    if let Some(package) = &document.identity.package {
        write!(output, "  package: {}", package.name)?;

        if let Some(version) = &package.version {
            write!(output, " {version}")?;
        }

        writeln!(output)?;

        if let Some(manifest_path) = &package.manifest_path {
            writeln!(output, "  manifest: {manifest_path}")?;
        }
    }

    if let Some(binary) = &document.identity.binary {
        writeln!(output, "  binary: {}", binary.name)?;
    }

    if let Some(source) = &document.identity.source {
        writeln!(output, "  source: {}", source_location(source))?;
    }

    Ok(())
}

fn write_provenance(
    provenance: Option<&Provenance>,
    output: &mut impl io::Write,
) -> io::Result<()> {
    let Some(provenance) = provenance else {
        return Ok(());
    };

    if let Some(owner) = &provenance.owner {
        writeln!(output, "    owner: {owner}")?;
    }

    if let Some(origin) = &provenance.origin {
        writeln!(output, "    origin: {origin}")?;
    }

    if let Some(ordinal) = provenance.ordinal {
        writeln!(output, "    ordinal: {ordinal}")?;
    }

    if let Some(source) = &provenance.source {
        writeln!(output, "    source: {}", source_location(source))?;
    }

    Ok(())
}

fn write_labels(labels: &BTreeMap<String, String>, output: &mut impl io::Write) -> io::Result<()> {
    for (name, value) in labels {
        writeln!(output, "    {name}: {value}")?;
    }

    Ok(())
}

fn write_facets(
    facets: &BTreeMap<String, Facet>,
    output: &mut impl io::Write,
    indentation: &str,
) -> io::Result<()> {
    for (namespace, facet) in facets {
        let value = serde_json::to_string(&facet.value).map_err(io::Error::other)?;

        writeln!(
            output,
            "{indentation}facet {namespace}@{}: {value}",
            facet.schema_version
        )?;
    }

    Ok(())
}

fn write_cli(
    document: &ToolingDocument,
    filters: &InspectFilters,
    color: bool,
    output: &mut impl io::Write,
) -> io::Result<()> {
    write_heading(output, "CLI", color)?;

    if !filters.kinds.is_empty() || !filters.facets.is_empty() || !filters.scopes.is_empty() {
        writeln!(output, "  filtered out")?;

        return Ok(());
    }

    let Some(cli) = &document.cli else {
        writeln!(output, "  disabled")?;

        return Ok(());
    };

    if filters.is_empty()
        && let Some(default_command) = &cli.default_command
    {
        writeln!(output, "  default: {default_command}")?;
    }

    let provider_ids = cli
        .providers
        .iter()
        .filter(|provider| matches_cli_provider(document, provider, filters))
        .map(|provider| provider.id.as_str())
        .collect::<BTreeSet<_>>();

    if !filters.is_empty() && provider_ids.is_empty() {
        writeln!(output, "  none")?;

        return Ok(());
    }

    for provider in cli
        .providers
        .iter()
        .filter(|provider| provider_ids.contains(provider.id.as_str()))
    {
        writeln!(
            output,
            "  provider {} [{}] contributor={} contribution={}",
            provider.id,
            cli_provider_kind_name(provider.kind),
            provider.contributor,
            provider.contribution
        )?;
    }

    write_cli_command(&cli.root, 1, &provider_ids, filters.is_empty(), output)
}

fn matches_cli_provider(
    document: &ToolingDocument,
    provider: &overseerd_tooling_schema::CliProvider,
    filters: &InspectFilters,
) -> bool {
    let kind_matches = filters.cli_provider_kinds.is_empty()
        || filters
            .cli_provider_kinds
            .iter()
            .any(|kind| cli_provider_kind_matches(provider.kind, *kind));
    let contributor = document
        .resources
        .iter()
        .find(|resource| resource.id == provider.contributor);
    let contribution_id = format!(
        "contribution:{}:{}",
        provider.contributor, provider.contribution
    );
    let contribution = document
        .resources
        .iter()
        .find(|resource| resource.id == contribution_id);
    let resource_matches = filters.resources.is_empty()
        || filters.resources.iter().any(|filter| {
            filter == &provider.id
                || resource_matches_filter(contributor, filter)
                || resource_matches_filter(contribution, filter)
        });
    let contributor_matches = filters.contributors.is_empty()
        || filters.contributors.iter().any(|filter| {
            filter == &provider.contributor
                || contributor.is_some_and(|resource| filter == &resource.name)
        });
    let plugin_matches = filters.plugins.is_empty()
        || contributor.is_some_and(|resource| {
            matches!(resource.kind, ResourceKind::Plugin)
                && filters
                    .plugins
                    .iter()
                    .any(|filter| filter == &resource.id || filter == &resource.name)
        });

    kind_matches && resource_matches && contributor_matches && plugin_matches
}

fn resource_matches_filter(resource: Option<&Resource>, filter: &str) -> bool {
    resource.is_some_and(|resource| filter == resource.id || filter == resource.name)
}

fn write_cli_command(
    command: &CliCommand,
    depth: usize,
    provider_ids: &BTreeSet<&str>,
    include_all: bool,
    output: &mut impl io::Write,
) -> io::Result<()> {
    let indentation = "  ".repeat(depth);

    if !include_all && depth > 1 && !command_uses_provider(command, provider_ids) {
        return Ok(());
    }

    writeln!(
        output,
        "{indentation}command {} [{}]",
        command.name,
        cli_owner_name(&command.owner)
    )?;

    for argument in command
        .arguments
        .iter()
        .filter(|argument| include_all || owner_uses_provider(&argument.owner, provider_ids))
    {
        write_cli_argument(argument, depth + 1, output)?;
    }

    for child in &command.commands {
        write_cli_command(child, depth + 1, provider_ids, include_all, output)?;
    }

    Ok(())
}

fn command_uses_provider(command: &CliCommand, provider_ids: &BTreeSet<&str>) -> bool {
    owner_uses_provider(&command.owner, provider_ids)
        || command
            .arguments
            .iter()
            .any(|argument| owner_uses_provider(&argument.owner, provider_ids))
        || command
            .commands
            .iter()
            .any(|command| command_uses_provider(command, provider_ids))
}

fn owner_uses_provider(
    owner: &overseerd_tooling_schema::CliOwner,
    provider_ids: &BTreeSet<&str>,
) -> bool {
    match owner {
        overseerd_tooling_schema::CliOwner::Plugin { provider } => {
            provider_ids.contains(provider.as_str())
        }
        _ => false,
    }
}

fn write_cli_argument(
    argument: &CliArgument,
    depth: usize,
    output: &mut impl io::Write,
) -> io::Result<()> {
    let indentation = "  ".repeat(depth);

    write!(output, "{indentation}argument {}", argument.id)?;

    if let Some(long) = &argument.long {
        write!(output, " --{long}")?;
    }

    if let Some(short) = argument.short {
        write!(output, " -{short}")?;
    }

    if let Some(index) = argument.index {
        write!(output, " positional={index}")?;
    }

    writeln!(output, " [{}]", cli_owner_name(&argument.owner))
}

fn write_diagnostics(
    diagnostics: &[Diagnostic],
    color: bool,
    output: &mut impl io::Write,
) -> io::Result<()> {
    write_heading(output, "Diagnostics", color)?;

    if diagnostics.is_empty() {
        writeln!(output, "  none")?;

        return Ok(());
    }

    for diagnostic in diagnostics {
        writeln!(
            output,
            "  {}[{}]: {}",
            diagnostic_severity_name(diagnostic.severity),
            diagnostic.code,
            diagnostic.message
        )?;

        if !diagnostic.resources.is_empty() {
            writeln!(output, "    resources: {}", diagnostic.resources.join(", "))?;
        }

        for source in &diagnostic.sources {
            writeln!(output, "    source: {}", source_location(source))?;
        }

        if let Some(fix) = &diagnostic.fix {
            writeln!(output, "    fix: {fix}")?;
        }
    }

    Ok(())
}

fn write_heading(output: &mut impl io::Write, heading: &str, color: bool) -> io::Result<()> {
    if color {
        writeln!(output, "\u{1b}[1;36m{heading}\u{1b}[0m")
    } else {
        writeln!(output, "{heading}")
    }
}

fn source_location(source: &SourceLocation) -> String {
    match (source.line, source.column) {
        (Some(line), Some(column)) => format!("{}:{line}:{column}", source.file),
        (Some(line), None) => format!("{}:{line}", source.file),
        _ => source.file.clone(),
    }
}

#[cfg(test)]
mod tests;
