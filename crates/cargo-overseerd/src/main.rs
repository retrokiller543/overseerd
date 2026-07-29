mod cli;
mod output;
mod render;

use std::io;
use std::path::Path;
use std::process::ExitCode;

use cargo_overseerd::{
    CancellationToken, CommandExitCode, ProbeRequestError, probe_request_exit_code, run_command,
    run_probe,
};
use overseerd_tooling_schema::{ProbeEnvelope, ProbeOutcome, ToolingDocument};

use crate::cli::{
    Cli, CommandRequest, ExportFormat, InspectFilters, InspectFormat, TerminalPolicy,
};
use crate::output::{write_export, write_text_inspection};

fn main() -> ExitCode {
    execute(Cli::parse_cargo().into_request())
}

fn execute(request: CommandRequest) -> ExitCode {
    let cancellation = CancellationToken::default();

    match request {
        CommandRequest::Report {
            command,
            discovery,
            format,
        } => {
            let report = run_command(command, &discovery, &cancellation);
            let exit_code = report.exit_code().code();
            let result = render::write_report(&report, format, &mut std::io::stdout().lock());

            finish_output(result, ExitCode::from(exit_code), "command report")
        }
        CommandRequest::Inspect {
            discovery,
            format,
            filters,
            color,
            pager,
        } => {
            if format == InspectFormat::Json && !filters.is_empty() {
                eprintln!(
                    "cargo overseerd inspect filters require text output; canonical JSON is always the complete document"
                );

                return ExitCode::from(CommandExitCode::Misuse.code());
            }

            match run_probe(&discovery, &cancellation) {
                Ok(probe) => inspect(probe.probe.envelope, format, filters, color, pager),
                Err(error) => probe_error(error),
            }
        }
        CommandRequest::Export {
            discovery,
            format,
            output,
        } => match run_probe(&discovery, &cancellation) {
            Ok(probe) => export(probe.probe.envelope, format, output.as_deref()),
            Err(error) => probe_error(error),
        },
    }
}

fn inspect(
    envelope: ProbeEnvelope,
    format: InspectFormat,
    filters: InspectFilters,
    color: TerminalPolicy,
    pager: TerminalPolicy,
) -> ExitCode {
    let ProbeOutcome::Success { document } = envelope.outcome else {
        eprintln!("cargo overseerd inspect could not prepare an inspection document");

        return validation_failure();
    };
    let exit_code = document_exit_code(&document);
    let result = match format {
        InspectFormat::Text => write_text_inspection(&document, &filters, color, pager),
        InspectFormat::Json => write_document_json(&document, &mut io::stdout().lock()),
    };

    finish_output(result, exit_code, "inspection")
}

fn export(envelope: ProbeEnvelope, format: ExportFormat, output: Option<&Path>) -> ExitCode {
    let exit_code = if envelope.is_success() {
        envelope_document(&envelope)
            .map(document_exit_code)
            .unwrap_or_else(validation_failure)
    } else {
        validation_failure()
    };
    let result = match format {
        ExportFormat::Document => match envelope_document(&envelope) {
            Some(document) => write_export(output, |writer| write_document_json(document, writer)),
            None => {
                eprintln!("cargo overseerd export cannot emit a document from a failed probe");

                return validation_failure();
            }
        },
        ExportFormat::Envelope => write_export(output, |writer| {
            let json = envelope.to_json().map_err(io::Error::other)?;

            writeln!(writer, "{json}")
        }),
    };

    finish_output(result, exit_code, "export")
}

fn finish_output(result: io::Result<()>, exit_code: ExitCode, output: &str) -> ExitCode {
    match result {
        Ok(()) => exit_code,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => exit_code,
        Err(error) => {
            eprintln!("cargo overseerd could not write its {output}: {error}");

            operational_failure()
        }
    }
}

fn write_document_json(document: &ToolingDocument, output: &mut dyn io::Write) -> io::Result<()> {
    let json = document.to_canonical_json().map_err(io::Error::other)?;

    writeln!(output, "{json}")
}

fn envelope_document(envelope: &ProbeEnvelope) -> Option<&ToolingDocument> {
    match &envelope.outcome {
        ProbeOutcome::Success { document } => Some(document),
        ProbeOutcome::Failure { .. } => None,
    }
}

fn document_exit_code(document: &ToolingDocument) -> ExitCode {
    if document.validation.valid {
        ExitCode::SUCCESS
    } else {
        validation_failure()
    }
}

fn probe_error(error: ProbeRequestError) -> ExitCode {
    let exit_code = probe_request_exit_code(&error);

    eprintln!("cargo overseerd could not read the application tooling probe: {error}");

    ExitCode::from(exit_code.code())
}

fn validation_failure() -> ExitCode {
    ExitCode::from(CommandExitCode::ValidationFailure.code())
}

fn operational_failure() -> ExitCode {
    ExitCode::from(CommandExitCode::OperationalFailure.code())
}
