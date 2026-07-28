use std::ffi::OsString;

use clap::Parser;

use super::{Cli, OutputFormat, normalized_arguments};
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
    let (command, _, format) = cli.into_request();

    assert_eq!(command, CommandKind::Check);
    assert_eq!(format, OutputFormat::Json);
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
    let (command, request, format) = cli.into_request();

    assert_eq!(command, CommandKind::Doctor);
    assert_eq!(request.package.as_deref(), Some("fixture"));
    assert_eq!(request.binary.as_deref(), Some("fixture-bin"));
    assert_eq!(request.features.features, ["tooling", "cli"]);
    assert_eq!(format, OutputFormat::Terminal);
}
