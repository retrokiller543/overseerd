use std::ffi::OsString;

use clap::{CommandFactory as _, Parser};

use super::{
    Cli, CommandRequest, ExportFormat, InspectFormat, InspectResourceKind, ReportFormat,
    TerminalPolicy, normalized_arguments,
};
use cargo_overseerd::CommandKind;

#[test]
fn cargo_external_subcommand_name_is_removed_before_parsing() {
    let arguments = normalized_arguments([
        OsString::from("cargo-overseerd"),
        OsString::from("overseerd"),
        OsString::from("check"),
        OsString::from("--format"),
        OsString::from("json"),
    ]);
    let cli = Cli::try_parse_from(arguments).expect("Cargo-style arguments parse");
    let CommandRequest::Report {
        command, format, ..
    } = cli.into_request()
    else {
        panic!("check produces a report request");
    };

    assert_eq!(command, CommandKind::Check);
    assert_eq!(format, ReportFormat::Json);
}

#[test]
fn direct_binary_arguments_remain_supported() {
    let arguments = normalized_arguments([
        OsString::from("cargo-overseerd"),
        OsString::from("doctor"),
        OsString::from("--package"),
        OsString::from("fixture"),
        OsString::from("--bin"),
        OsString::from("fixture-bin"),
        OsString::from("--features"),
        OsString::from("tooling,cli"),
    ]);
    let cli = Cli::try_parse_from(arguments).expect("direct arguments parse");
    let CommandRequest::Report {
        command,
        discovery: request,
        format,
    } = cli.into_request()
    else {
        panic!("doctor produces a report request");
    };

    assert_eq!(command, CommandKind::Doctor);
    assert_eq!(request.package.as_deref(), Some("fixture"));
    assert_eq!(request.binary.as_deref(), Some("fixture-bin"));
    assert_eq!(request.features.features, ["tooling", "cli"]);
    assert_eq!(format, ReportFormat::Terminal);
}

#[test]
fn inspect_parses_filters_and_terminal_policy() {
    let cli = Cli::try_parse_from([
        "cargo-overseerd",
        "inspect",
        "--format",
        "text",
        "--kind",
        "component,provider",
        "--resource",
        "Worker",
        "--color",
        "never",
        "--pager",
        "always",
    ])
    .expect("inspect arguments parse");
    let CommandRequest::Inspect {
        format,
        filters,
        color,
        pager,
        ..
    } = cli.into_request()
    else {
        panic!("inspect produces an inspect request");
    };

    assert_eq!(format, InspectFormat::Text);
    assert_eq!(
        filters.kinds,
        [
            InspectResourceKind::Component,
            InspectResourceKind::Provider
        ]
    );
    assert_eq!(filters.resources, ["Worker"]);
    assert_eq!(color, TerminalPolicy::Never);
    assert_eq!(pager, TerminalPolicy::Always);
}

#[test]
fn export_parses_payload_and_output_path() {
    let cli = Cli::try_parse_from([
        "cargo-overseerd",
        "export",
        "--format",
        "envelope",
        "--output",
        "inspection.json",
    ])
    .expect("export arguments parse");
    let CommandRequest::Export { format, output, .. } = cli.into_request() else {
        panic!("export produces an export request");
    };

    assert_eq!(format, ExportFormat::Envelope);
    assert_eq!(
        output.as_deref(),
        Some(std::path::Path::new("inspection.json"))
    );
}

#[test]
fn cargo_help_uses_external_subcommand_invocation_name() {
    let mut command = Cli::command();
    let mut output = Vec::new();

    command
        .write_long_help(&mut output)
        .expect("long help writes");

    let output = String::from_utf8(output).expect("help is UTF-8");

    assert!(output.contains("Usage: cargo overseerd <COMMAND>"));
}

#[test]
fn cargo_help_uses_colored_cargo_style_headings_and_literals() {
    let styles = Cli::command().get_styles().clone();

    assert_eq!(styles.get_header().to_string(), "\u{1b}[1m\u{1b}[92m");
    assert_eq!(styles.get_literal().to_string(), "\u{1b}[1m\u{1b}[96m");
}
