use std::collections::BTreeMap;
use std::io;

use overseerd_tooling_schema::{Diagnostic, Facet, Provenance, SourceLocation, ToolingDocument};

use super::name::diagnostic_severity_name;

pub(crate) fn write_heading(
    output: &mut dyn io::Write,
    heading: &str,
    color: bool,
) -> io::Result<()> {
    let heading = terminal_text(heading);

    if color {
        writeln!(output, "\u{1b}[1;36m{heading}\u{1b}[0m")
    } else {
        writeln!(output, "{heading}")
    }
}

pub(crate) fn write_diagnostics(
    diagnostics: &[Diagnostic],
    indentation: &str,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    for diagnostic in diagnostics {
        let code = terminal_text(&diagnostic.code);
        let message = terminal_text(&diagnostic.message);

        writeln!(
            output,
            "{indentation}{}[{}]: {}",
            diagnostic_severity_name(diagnostic.severity),
            code,
            message
        )?;

        if !diagnostic.resources.is_empty() {
            let resources = diagnostic
                .resources
                .iter()
                .map(|resource| terminal_text(resource))
                .collect::<Vec<_>>()
                .join(", ");

            writeln!(output, "{indentation}  resources: {}", resources)?;
        }

        for source in &diagnostic.sources {
            writeln!(output, "{indentation}  source: {}", source_location(source))?;
        }

        if let Some(fix) = &diagnostic.fix {
            writeln!(output, "{indentation}  fix: {}", terminal_text(fix))?;
        }
    }

    Ok(())
}

pub(crate) fn source_location(source: &SourceLocation) -> String {
    let file = terminal_text(&source.file);

    match (source.line, source.column) {
        (Some(line), Some(column)) => format!("{file}:{line}:{column}"),
        (Some(line), None) => format!("{file}:{line}"),
        _ => file,
    }
}

pub(crate) fn terminal_text(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());

    for character in value.chars() {
        match character {
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write as _;

                write!(escaped, "\\u{{{:x}}}", character as u32)
                    .expect("writing to a string cannot fail");
            }
            character => escaped.push(character),
        }
    }

    escaped
}

pub(crate) fn write_identity(
    document: &ToolingDocument,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    if let Some(package) = &document.identity.package {
        write!(output, "  package: {}", terminal_text(&package.name))?;

        if let Some(version) = &package.version {
            write!(output, " {}", terminal_text(version))?;
        }

        writeln!(output)?;

        if let Some(manifest_path) = &package.manifest_path {
            writeln!(output, "  manifest: {}", terminal_text(manifest_path))?;
        }
    }

    if let Some(binary) = &document.identity.binary {
        writeln!(output, "  binary: {}", terminal_text(&binary.name))?;
    }

    if let Some(source) = &document.identity.source {
        writeln!(output, "  source: {}", source_location(source))?;
    }

    Ok(())
}

pub(crate) fn write_provenance(
    provenance: Option<&Provenance>,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    let Some(provenance) = provenance else {
        return Ok(());
    };

    if let Some(owner) = &provenance.owner {
        writeln!(output, "    owner: {}", terminal_text(owner))?;
    }

    if let Some(origin) = &provenance.origin {
        writeln!(output, "    origin: {}", terminal_text(origin))?;
    }

    if let Some(ordinal) = provenance.ordinal {
        writeln!(output, "    ordinal: {ordinal}")?;
    }

    if let Some(source) = &provenance.source {
        writeln!(output, "    source: {}", source_location(source))?;
    }

    Ok(())
}

pub(crate) fn write_labels(
    labels: &BTreeMap<String, String>,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    for (name, value) in labels {
        writeln!(
            output,
            "    {}: {}",
            terminal_text(name),
            terminal_text(value)
        )?;
    }

    Ok(())
}

pub(crate) fn write_facets(
    facets: &BTreeMap<String, Facet>,
    output: &mut dyn io::Write,
    indentation: &str,
) -> io::Result<()> {
    for (namespace, facet) in facets {
        let value = serde_json::to_string(&facet.value).map_err(io::Error::other)?;

        writeln!(
            output,
            "{indentation}facet {}@{}: {value}",
            terminal_text(namespace),
            facet.schema_version
        )?;
    }

    Ok(())
}
