use std::io;

use cargo_overseerd::{CommandCheckStatus, CommandOutcome, CommandReport};
use overseerd_tooling_schema::DiagnosticSeverity;

use crate::cli::OutputFormat;

pub(crate) fn write_report(
    report: &CommandReport,
    format: OutputFormat,
    stdout: &mut impl io::Write,
) -> io::Result<()> {
    match format {
        OutputFormat::Terminal => write_terminal(report, stdout),
        OutputFormat::Json => {
            let json = report.to_json().map_err(io::Error::other)?;

            writeln!(stdout, "{json}")
        }
    }
}

fn write_terminal(report: &CommandReport, output: &mut impl io::Write) -> io::Result<()> {
    if let Some(application) = &report.application {
        write!(output, "Application {}", application.application)?;

        if let Some(protocol) = &report.protocol {
            write!(output, " ({protocol})")?;
        }

        writeln!(output)?;
    }

    if let Some(target) = &report.target {
        writeln!(
            output,
            "Target {} {} / {}",
            target.package, target.version, target.binary
        )?;
    }

    if let Some(version) = &report.framework_version {
        writeln!(output, "Framework Overseerd {version}")?;
    }

    for check in &report.checks {
        let status = match check.status {
            CommandCheckStatus::Passed => "PASS",
            CommandCheckStatus::Warning => "WARN",
            CommandCheckStatus::Failed => "FAIL",
        };

        writeln!(output, "[{status}] {}: {}", check.code, check.message)?;
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
            diagnostic.code, diagnostic.message
        )?;

        if !diagnostic.resources.is_empty() {
            writeln!(output, "  resources: {}", diagnostic.resources.join(", "))?;
        }

        for source in &diagnostic.sources {
            match (source.line, source.column) {
                (Some(line), Some(column)) => {
                    writeln!(output, "  at {}:{line}:{column}", source.file)?;
                }
                _ => writeln!(output, "  at {}", source.file)?,
            }
        }

        if let Some(fix) = &diagnostic.fix {
            writeln!(output, "  fix: {fix}")?;
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
