mod cli;
mod output;
mod render;

use std::io::{self, IsTerminal as _};
use std::path::Path;
use std::process::ExitCode;

use cargo_upwell::{
    CancellationToken, Catalog, CommandExitCode, GraphQuery, GraphQueryError, InitError,
    ProbeOptions, ProbeRequestError, ResourceExplanation, TemplateSelection, TemplateSource,
    ToolingProbe, init_project, probe_request_exit_code, run_command_with_options,
    run_probe_with_options,
};
use upwell_tooling_schema::{Diagnostic, ProbeEnvelope, ProbeOutcome, ToolingDocument};

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
        CommandRequest::Init(mut request) => {
            if let TemplateSelection::Catalog {
                template: None,
                catalog_path,
            } = &request.template
            {
                match select_template(catalog_path.as_deref()) {
                    Ok(template) => {
                        request.template = TemplateSelection::Catalog {
                            template: Some(template),
                            catalog_path: catalog_path.clone(),
                        };
                    }
                    Err(error) => return init_error(error),
                }
            }

            match init_project(request) {
                Ok(result) => {
                    println!("Created {} from {}", result.path.display(), result.template);

                    ExitCode::SUCCESS
                }
                Err(error) => init_error(error),
            }
        }
        CommandRequest::Templates { catalog_path } => {
            match Catalog::load(catalog_path.as_deref()) {
                Ok(catalog) => {
                    print_templates(&catalog);

                    ExitCode::SUCCESS
                }
                Err(error) => init_error(InitError::Catalog(error)),
            }
        }
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
        } => {
            if format == InspectFormat::Json && !filters.is_empty() {
                eprintln!(
                    "cargo upwell inspect filters require text output; canonical JSON is always the complete document"
                );

                return ExitCode::from(CommandExitCode::Misuse.code());
            }

            let options = ProbeOptions {
                show_cargo_output: interactive,
            };

            match run_probe_with_options(&discovery, &cancellation, options) {
                Ok(probe) => inspect(probe, format, filters, color, pager),
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
        } => match run_probe_with_options(
            &discovery,
            &cancellation,
            ProbeOptions {
                show_cargo_output: interactive,
            },
        ) {
            Ok(probe) => graph(probe, format, query, color, pager),
            Err(error) => probe_error(error),
        },
        CommandRequest::Explain {
            discovery,
            format,
            resource,
            color,
            pager,
        } => match run_probe_with_options(
            &discovery,
            &cancellation,
            ProbeOptions {
                show_cargo_output: interactive,
            },
        ) {
            Ok(probe) => explain(probe, format, &resource, color, pager),
            Err(error) => probe_error(error),
        },
    }
}

fn init_error(error: InitError) -> ExitCode {
    eprintln!("cargo upwell could not initialize the project: {error}");

    match error {
        InitError::MissingName
        | InitError::ReservedValue(_)
        | InitError::InvalidTemplatePath(_)
        | InitError::DestinationExists(_)
        | InitError::MissingTemplate => ExitCode::from(CommandExitCode::Misuse.code()),
        InitError::Catalog(cargo_upwell::CatalogError::Read { .. }) => operational_failure(),
        InitError::Catalog(_) => ExitCode::from(CommandExitCode::Misuse.code()),
        _ => operational_failure(),
    }
}

fn select_template(path: Option<&Path>) -> Result<String, InitError> {
    let catalog = Catalog::load(path)?;
    let templates = catalog.templates().collect::<Vec<_>>();

    if !io::stderr().is_terminal() {
        eprintln!("Template selection requires an interactive terminal.");
        write_templates(&catalog, &mut io::stderr().lock()).map_err(|source| {
            InitError::Generate {
                template: String::from("catalog"),
                source: anyhow::Error::new(source),
            }
        })?;
        eprintln!("Pass --template <ID>, or use --template-path <PATH>.");

        return Err(InitError::MissingTemplate);
    }

    let labels = templates
        .iter()
        .map(|template| format!("{}  {}", template.id(), template.description()))
        .collect::<Vec<_>>();
    let selection = dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt("Choose an Upwell project template")
        .items(&labels)
        .default(0)
        .max_length(12)
        .interact_opt()
        .map_err(|source| InitError::Generate {
            template: String::from("selector"),
            source: anyhow::Error::new(source),
        })?
        .ok_or(InitError::MissingTemplate)?;

    Ok(templates[selection].id().to_owned())
}

fn print_templates(catalog: &Catalog) {
    if let Err(error) = write_templates(catalog, &mut io::stdout().lock()) {
        eprintln!("cargo upwell could not write the template catalog: {error}");
    }
}

fn write_templates(catalog: &Catalog, output: &mut dyn io::Write) -> io::Result<()> {
    for template in catalog.templates() {
        let source = match template.source() {
            TemplateSource::Local(path) => format!("path {}", path.display()),
            TemplateSource::Git {
                repository,
                reference,
            } => reference.as_ref().map_or_else(
                || format!("git {repository}"),
                |reference| format!("git {repository} ({reference})"),
            ),
        };

        writeln!(
            output,
            "{:<24} {:<54} {}",
            template.id(),
            template.description(),
            source
        )?;
    }

    Ok(())
}

fn inspect(
    probe: ToolingProbe,
    format: InspectFormat,
    filters: InspectFilters,
    color: TerminalPolicy,
    pager: TerminalPolicy,
) -> ExitCode {
    let envelope = probe.probe.envelope;
    let document = match envelope.outcome {
        ProbeOutcome::Success { document } => document,
        ProbeOutcome::Failure { failure } => {
            return preparation_failure("inspect", &failure.diagnostics);
        }
    };
    let exit_code = document_exit_code(&document);
    let result = match format {
        InspectFormat::Text => write_text(color, pager, |color, output| {
            render::write_inspection(&document, &filters, color, output)
        }),
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
) -> ExitCode {
    let envelope = probe.probe.envelope;
    let (view, exit_code) = match envelope.outcome {
        ProbeOutcome::Success { document } => {
            let view = match query.execute(&document) {
                Ok(view) => view,
                Err(error) => return graph_query_error("graph", error),
            };
            let exit_code = document_exit_code(&document);

            (view, exit_code)
        }
        ProbeOutcome::Failure { failure } => {
            let view = match query.execute_failure(envelope.schema, &envelope.identity, &failure) {
                Ok(view) => view,
                Err(error) => return graph_query_error("graph", error),
            };

            (view, validation_failure())
        }
    };
    let result = if format == GraphFormat::Text {
        write_text(color, pager, |color, output| {
            render::write_graph(&view, format, color, output)
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
) -> ExitCode {
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
        write_text(color, pager, |color, output| {
            render::write_explanation(&explanation, format, color, output)
        })
    } else {
        render::write_explanation(&explanation, format, false, &mut io::stdout().lock())
    };

    finish_output(result, exit_code, "explanation")
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
                eprintln!("cargo upwell export cannot emit a document from a failed probe");

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
            eprintln!("cargo upwell could not write its {output}: {error}");

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

    eprintln!("cargo upwell could not read the application tooling probe: {error}");

    ExitCode::from(exit_code.code())
}

fn preparation_failure(command: &str, diagnostics: &[Diagnostic]) -> ExitCode {
    let result = render::write_diagnostics(diagnostics, "", &mut io::stderr().lock());

    if let Err(error) = result
        && error.kind() != io::ErrorKind::BrokenPipe
    {
        eprintln!("cargo upwell could not write {command} failure diagnostics: {error}");

        return operational_failure();
    }

    validation_failure()
}

fn graph_query_error(command: &str, error: GraphQueryError) -> ExitCode {
    let command = render::terminal_text(command);
    let error_text = render::terminal_text(&error.to_string());

    eprintln!("cargo upwell {command}: {error_text}");

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
