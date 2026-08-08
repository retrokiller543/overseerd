use std::process::ExitCode;

fn main() -> ExitCode {
    match cargo_upwell::run_component_compiler_worker() {
        Some(Ok(())) => ExitCode::SUCCESS,
        Some(Err(error)) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
        None => {
            eprintln!("renderer compiler worker requires a request from cargo-upwell");
            ExitCode::FAILURE
        }
    }
}
