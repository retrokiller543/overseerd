mod cli;
mod output;
mod render;

use std::io::{self, IsTerminal as _};
use std::path::Path;
use std::process::ExitCode;

use cargo_overseerd::{
    CancellationToken, CommandExitCode, GraphQuery, GraphQueryError, ProbeOptions,
    ProbeRequestError, RendererRun, ResourceExplanation, ToolingProbe, load_renderers,
    probe_request_exit_code, run_command_with_options, run_probe_with_options, run_renderers,
};
use overseerd_tooling_schema::renderer::RendererView;
use overseerd_tooling_schema::{Diagnostic, ProbeEnvelope, ProbeOutcome, ToolingDocument};

use crate::cli::{
    Cli, CommandRequest, ExplainFormat, ExportFormat, GraphFormat, InspectFilters, InspectFormat,
    TerminalPolicy,
};
use crate::output::{write_export, write_text};

fn main() -> ExitCode {
    execute(Cli::parse_cargo().into_request())
}

fn execute(request: CommandRequest) -> ExitCode {
    let cancellation = CancellationToken::default();
    let interactive = io::stderr().is_terminal();

    match request {
        CommandRequest::Report {
            command,
            discovery,
            format,
        } => {
            let options = ProbeOptions {
                show_cargo_output: interactive,
            };
            let report = run_command_with_options(command, &discovery, &cancellation, options);
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
            renderers,
        } => {
            if format == InspectFormat::Json && !filters.is_empty() {
                eprintln!(
                    "cargo overseerd inspect filters require text output; canonical JSON is always the complete document"
                );

                return ExitCode::from(CommandExitCode::Misuse.code());
            }

            let options = ProbeOptions {
                show_cargo_output: interactive,
            };

            match run_probe_with_options(&discovery, &cancellation, options) {
                Ok(probe) => inspect(
                    probe,
                    format,
                    filters,
                    color,
                    pager,
                    renderers,
                    &cancellation,
                ),
                Err(error) => probe_error(error),
            }
        }
        CommandRequest::Export {
            discovery,
            format,
            output,
        } => match run_probe_with_options(
            &discovery,
            &cancellation,
            ProbeOptions {
                show_cargo_output: interactive,
            },
        ) {
            Ok(probe) => export(probe.probe.envelope, format, output.as_deref()),
            Err(error) => probe_error(error),
        },
        CommandRequest::Graph {
            discovery,
            format,
            query,
            color,
            pager,
            renderers,
        } => match run_probe_with_options(
            &discovery,
            &cancellation,
            ProbeOptions {
                show_cargo_output: interactive,
            },
        ) {
            Ok(probe) => graph(probe, format, query, color, pager, renderers, &cancellation),
            Err(error) => probe_error(error),
        },
        CommandRequest::Explain {
            discovery,
            format,
            resource,
            color,
            pager,
            renderers,
        } => match run_probe_with_options(
            &discovery,
            &cancellation,
            ProbeOptions {
                show_cargo_output: interactive,
            },
        ) {
            Ok(probe) => explain(
                probe,
                format,
                &resource,
                color,
                pager,
                renderers,
                &cancellation,
            ),
            Err(error) => probe_error(error),
        },
    }
}

fn inspect(
    probe: ToolingProbe,
    format: InspectFormat,
    filters: InspectFilters,
    color: TerminalPolicy,
    pager: TerminalPolicy,
    renderers: Vec<std::path::PathBuf>,
    cancellation: &CancellationToken,
) -> ExitCode {
    let target_directory = probe.workspace.target_directory;
    let envelope = probe.probe.envelope;
    let document = match envelope.outcome {
        ProbeOutcome::Success { document } => document,
        ProbeOutcome::Failure { failure } => {
            return preparation_failure("inspect", &failure.diagnostics);
        }
    };
    let exit_code = document_exit_code(&document);
    let result = match format {
        InspectFormat::Text => {
            let resources = render::selected_inspection_resource_ids(&document, &filters);
            let renderer_run = renderer_run(
                renderers,
                &document,
                RendererView::Inspect,
                resources,
                &target_directory,
                cancellation,
            );

            write_renderer_diagnostics(&renderer_run);

            write_text(color, pager, |color, output| {
                render::write_inspection_with_presentation(
                    &document,
                    &filters,
                    Some(&renderer_run.presentation),
                    color,
                    output,
                )
            })
        }
        InspectFormat::Json => write_document_json(&document, &mut io::stdout().lock()),
    };

    finish_output(result, exit_code, "inspection")
}

fn graph(
    probe: ToolingProbe,
    format: GraphFormat,
    query: GraphQuery,
    color: TerminalPolicy,
    pager: TerminalPolicy,
    renderers: Vec<std::path::PathBuf>,
    cancellation: &CancellationToken,
) -> ExitCode {
    let target_directory = probe.workspace.target_directory;
    let envelope = probe.probe.envelope;
    let (view, document, exit_code) = match envelope.outcome {
        ProbeOutcome::Success { document } => {
            let view = match query.execute(&document) {
                Ok(view) => view,
                Err(error) => return graph_query_error("graph", error),
            };
            let exit_code = document_exit_code(&document);

            (view, Some(document), exit_code)
        }
        ProbeOutcome::Failure { failure } => {
            let view = match query.execute_failure(envelope.schema, &envelope.identity, &failure) {
                Ok(view) => view,
                Err(error) => return graph_query_error("graph", error),
            };

            (view, None, validation_failure())
        }
    };
    let result = if format == GraphFormat::Text {
        let renderer_run = document
            .as_deref()
            .map(|document| {
                renderer_run(
                    renderers,
                    document,
                    RendererView::Graph,
                    view.nodes
                        .iter()
                        .map(|node| node.id.clone())
                        .collect::<Vec<_>>(),
                    &target_directory,
                    cancellation,
                )
            })
            .unwrap_or_default();

        write_renderer_diagnostics(&renderer_run);

        write_text(color, pager, |color, output| {
            render::write_graph_with_presentation(
                &view,
                format,
                Some(&renderer_run.presentation),
                color,
                output,
            )
        })
    } else {
        render::write_graph(&view, format, false, &mut io::stdout().lock())
    };

    finish_output(result, exit_code, "graph")
}

fn explain(
    probe: ToolingProbe,
    format: ExplainFormat,
    resource: &str,
    color: TerminalPolicy,
    pager: TerminalPolicy,
    renderers: Vec<std::path::PathBuf>,
    cancellation: &CancellationToken,
) -> ExitCode {
    let target_directory = probe.workspace.target_directory;
    let envelope = probe.probe.envelope;
    let document = match envelope.outcome {
        ProbeOutcome::Success { document } => document,
        ProbeOutcome::Failure { failure } => {
            return preparation_failure("explain", &failure.diagnostics);
        }
    };
    let explanation = match ResourceExplanation::query(&document, resource) {
        Ok(explanation) => explanation,
        Err(error) => return graph_query_error("explain", error),
    };
    let exit_code = document_exit_code(&document);
    let result = if format == ExplainFormat::Text {
        let renderer_run = renderer_run(
            renderers,
            &document,
            RendererView::Explain,
            [explanation.resource.id.clone()],
            &target_directory,
            cancellation,
        );

        write_renderer_diagnostics(&renderer_run);

        write_text(color, pager, |color, output| {
            render::write_explanation_with_presentation(
                &explanation,
                format,
                Some(&renderer_run.presentation),
                color,
                output,
            )
        })
    } else {
        render::write_explanation(&explanation, format, false, &mut io::stdout().lock())
    };

    finish_output(result, exit_code, "explanation")
}

fn renderer_run(
    manifests: Vec<std::path::PathBuf>,
    document: &ToolingDocument,
    view: RendererView,
    resources: impl IntoIterator<Item = String> + Clone,
    target_directory: &Path,
    cancellation: &CancellationToken,
) -> RendererRun {
    if manifests.is_empty() {
        return RendererRun::default();
    }

    let renderers = match load_renderers(manifests) {
        Ok(renderers) => renderers,
        Err(error) => {
            eprintln!(
                "warning[overseerd/renderer-manifest]: {}",
                render::terminal_text(&error.to_string())
            );

            return RendererRun::default();
        }
    };

    run_renderers(
        &renderers,
        document,
        view,
        resources,
        target_directory,
        cancellation,
    )
}

fn write_renderer_diagnostics(run: &RendererRun) {
    for diagnostic in &run.diagnostics {
        eprintln!(
            "warning[{}]: renderer {}: {}",
            diagnostic.code.as_str(),
            render::terminal_text(&diagnostic.renderer),
            render::terminal_text(&diagnostic.message)
        );
    }
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

fn preparation_failure(command: &str, diagnostics: &[Diagnostic]) -> ExitCode {
    let result = render::write_diagnostics(diagnostics, "", &mut io::stderr().lock());

    if let Err(error) = result
        && error.kind() != io::ErrorKind::BrokenPipe
    {
        eprintln!("cargo overseerd could not write {command} failure diagnostics: {error}");

        return operational_failure();
    }

    validation_failure()
}

fn graph_query_error(command: &str, error: GraphQueryError) -> ExitCode {
    let command = render::terminal_text(command);
    let error_text = render::terminal_text(&error.to_string());

    eprintln!("cargo overseerd {command}: {error_text}");

    if let GraphQueryError::Ambiguous { candidates, .. } = error {
        eprintln!("candidates:");

        for candidate in candidates {
            eprintln!("  {}", render::terminal_text(&candidate));
        }
    }

    ExitCode::from(CommandExitCode::Misuse.code())
}

fn validation_failure() -> ExitCode {
    ExitCode::from(CommandExitCode::ValidationFailure.code())
}

fn operational_failure() -> ExitCode {
    ExitCode::from(CommandExitCode::OperationalFailure.code())
}
