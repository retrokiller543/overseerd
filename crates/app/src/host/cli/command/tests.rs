use std::error::Error as _;

use super::{CommandContext, CommandContextError, CommandError};
use crate::{AppHost, BootstrapContext, ExecutionMode, Setup};

/// Host used to type command contexts without constructing an application.
struct TestHost;

impl AppHost for TestHost {
    type Protocol = ();

    fn builder() -> Result<crate::AppBuilder<Self::Protocol>, upwell_config::ConfigError> {
        Ok(crate::AppBuilder::new("test"))
    }
}

#[test]
fn setup_context_exposes_bootstrap_values() {
    let mut bootstrap = BootstrapContext::new(ExecutionMode::Run);

    bootstrap.insert(String::from("global"));

    let context = CommandContext::<TestHost, Setup>::new(bootstrap, ());

    assert_eq!(
        context.bootstrap().get::<String>().map(String::as_str),
        Some("global")
    );
    assert_eq!(
        context
            .require::<String>()
            .expect("string value is present"),
        "global"
    );
    assert!(matches!(
        context.require::<usize>(),
        Err(CommandContextError::MissingValue { type_name })
            if type_name == std::any::type_name::<usize>()
    ));
}

#[test]
fn command_error_preserves_path_and_source() {
    let error = CommandError::new("api users list", std::io::Error::other("offline"));

    assert_eq!(error.command(), "api users list");
    assert_eq!(
        error.to_string(),
        "command `api users list` failed: offline"
    );
    assert_eq!(
        error.source().map(ToString::to_string),
        Some(String::from("offline"))
    );
}
