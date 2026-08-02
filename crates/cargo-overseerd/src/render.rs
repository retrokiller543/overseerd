use std::io;

use cargo_overseerd::{CommandCheckStatus, CommandOutcome, CommandReport};
use overseerd_tooling_schema::DiagnosticSeverity;

use crate::cli::ReportFormat;
use detail::source_location;

mod detail;
mod explain;
mod graph;
mod inspect;
mod name;

pub(crate) use detail::{terminal_text, write_diagnostics};
pub(crate) use explain::write_explanation;
pub(crate) use explain::write_explanation_with_presentation;
pub(crate) use graph::write_graph;
pub(crate) use graph::write_graph_with_presentation;
pub(crate) use inspect::write_inspection_with_presentation;

pub(crate) fn write_report(
    report: &CommandReport,
    format: ReportFormat,
    stdout: &mut impl io::Write,
) -> io::Result<()> {
    match format {
        ReportFormat::Terminal => write_terminal(report, stdout),
        ReportFormat::Json => {
            let json = report.to_json().map_err(io::Error::other)?;

            writeln!(stdout, "{json}")
        }
    }
}

fn write_terminal(report: &CommandReport, output: &mut impl io::Write) -> io::Result<()> {
    if let Some(application) = &report.application {
        write!(
            output,
            "Application {}",
            terminal_text(&application.application)
        )?;

        if let Some(protocol) = &report.protocol {
            write!(output, " ({})", terminal_text(protocol))?;
        }

        writeln!(output)?;
    }

    if let Some(target) = &report.target {
        writeln!(
            output,
            "Target {} {} / {}",
            terminal_text(&target.package),
            terminal_text(&target.version),
            terminal_text(&target.binary)
        )?;
    }

    if let Some(version) = &report.framework_version {
        writeln!(output, "Framework Overseerd {}", terminal_text(version))?;
    }

    for check in &report.checks {
        let status = match check.status {
            CommandCheckStatus::Passed => "PASS",
            CommandCheckStatus::Warning => "WARN",
            CommandCheckStatus::Failed => "FAIL",
        };

        writeln!(
            output,
            "[{status}] {}: {}",
            terminal_text(&check.code),
            terminal_text(&check.message)
        )?;
    }

    for diagnostic in &report.diagnostics {
        let severity = match diagnostic.severity {
            DiagnosticSeverity::Info => "info",
            DiagnosticSeverity::Warning => "warning",
            DiagnosticSeverity::Error => "error",
            _ => "diagnostic",
        };

        writeln!(
            output,
            "{severity}[{}]: {}",
            terminal_text(&diagnostic.code),
            terminal_text(&diagnostic.message)
        )?;

        if !diagnostic.resources.is_empty() {
            let resources = diagnostic
                .resources
                .iter()
                .map(|resource| terminal_text(resource))
                .collect::<Vec<_>>()
                .join(", ");

            writeln!(output, "  resources: {resources}")?;
        }

        for source in &diagnostic.sources {
            writeln!(output, "  at {}", source_location(source))?;
        }

        if let Some(fix) = &diagnostic.fix {
            writeln!(output, "  fix: {}", terminal_text(fix))?;
        }
    }

    let summary = match report.outcome {
        CommandOutcome::Success => "passed",
        CommandOutcome::ValidationFailure => "found validation failures",
        CommandOutcome::TargetSelectionFailure => "could not select an application target",
        CommandOutcome::BuildFailure => "could not build the application target",
        CommandOutcome::ProbeFailure => "could not read the application tooling probe",
        CommandOutcome::OperationalFailure => "could not complete",
    };

    writeln!(
        output,
        "cargo overseerd {} {summary}",
        match report.command {
            cargo_overseerd::CommandKind::Check => "check",
            cargo_overseerd::CommandKind::Doctor => "doctor",
        }
    )?;
    output.flush()
}

#[cfg(test)]
mod tests;
