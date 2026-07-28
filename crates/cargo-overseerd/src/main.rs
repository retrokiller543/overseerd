mod cli;
mod render;

use std::process::ExitCode;

use cargo_overseerd::{CancellationToken, run_command};

use crate::cli::Cli;

fn main() -> ExitCode {
    let (command, request, format) = Cli::parse_cargo().into_request();
    let report = run_command(command, &request, &CancellationToken::default());
    let exit_code = report.exit_code().code();

    if let Err(error) = render::write_report(&report, format, &mut std::io::stdout().lock()) {
        eprintln!("cargo overseerd could not write its command report: {error}");

        return ExitCode::from(6);
    }

    ExitCode::from(exit_code)
}
