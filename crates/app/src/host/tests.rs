use std::error::Error as _;

use super::{BootstrapContext, ExecutionMode, LifecyclePhase, PhaseError};

#[cfg(feature = "cli")]
use super::{BootstrapOptions, ColorChoice};

#[cfg(feature = "cli")]
use crate::LogFormat;

#[test]
fn bootstrap_context_stores_values_by_type() {
    let mut context = BootstrapContext::new(ExecutionMode::Tooling);

    assert!(context.mode().is_tooling());
    assert_eq!(context.insert(String::from("first")), None);
    assert_eq!(context.get::<String>().map(String::as_str), Some("first"));
    assert_eq!(
        context.insert(String::from("second")),
        Some(String::from("first"))
    );

    context
        .get_mut::<String>()
        .expect("string extension exists")
        .push_str(" value");

    assert_eq!(
        context.remove::<String>(),
        Some(String::from("second value"))
    );
    assert!(context.get::<String>().is_none());
}

#[test]
fn phase_error_preserves_phase_and_source() {
    let error = PhaseError::new(LifecyclePhase::BeforeBuild, std::io::Error::other("failed"));

    assert_eq!(error.phase(), LifecyclePhase::BeforeBuild);
    assert_eq!(
        error.source().map(ToString::to_string),
        Some(String::from("failed"))
    );
    assert_eq!(error.to_string(), "before_build phase failed: failed");
}

#[cfg(feature = "cli")]
#[test]
fn bootstrap_options_parse_typed_global_values() {
    use clap::Parser as _;

    #[derive(clap::Parser)]
    struct TestCli {
        #[command(flatten)]
        bootstrap: BootstrapOptions,
    }

    let cli = TestCli::try_parse_from([
        "test",
        "--config",
        "config/application.toml",
        "--profile",
        "base",
        "-p",
        "local",
        "--log",
        "warn,overseerd=trace",
        "--log-format",
        "json",
        "--color",
        "always",
    ])
    .expect("bootstrap options parse");

    assert_eq!(
        cli.bootstrap.config(),
        Some(std::path::Path::new("config/application.toml"))
    );
    assert_eq!(cli.bootstrap.profiles(), ["base", "local"]);
    assert_eq!(cli.bootstrap.log(), Some("warn,overseerd=trace"));
    assert_eq!(cli.bootstrap.log_format(), Some(LogFormat::Json));
    assert_eq!(cli.bootstrap.color(), Some(ColorChoice::Always));
}

#[cfg(feature = "cli")]
#[test]
fn bootstrap_options_reject_unknown_typed_values() {
    use clap::Parser as _;

    #[derive(clap::Parser)]
    struct TestCli {
        #[command(flatten)]
        bootstrap: BootstrapOptions,
    }

    let result = TestCli::try_parse_from(["test", "--log-format", "xml"]);
    let error = match result {
        Ok(_) => panic!("unknown log format was accepted"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
}
