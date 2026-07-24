use std::error::Error as _;

use super::{CommandContext, CommandError, CommandPhase};
use crate::{AppHost, BootstrapContext, ExecutionMode};

/// Host used to type command contexts without constructing an application.
struct TestHost;

impl AppHost for TestHost {
    type Protocol = ();

    fn builder() -> Result<crate::AppBuilder<Self::Protocol>, overseerd_config::ConfigError> {
        Ok(crate::AppBuilder::new("test"))
    }
}

#[test]
fn setup_context_exposes_bootstrap_without_application_state() {
    let mut bootstrap = BootstrapContext::new(ExecutionMode::Run);

    bootstrap.insert(String::from("global"));

    let context = CommandContext::<TestHost>::from_setup(bootstrap);

    assert_eq!(context.phase(), CommandPhase::Setup);
    assert_eq!(
        context.bootstrap().get::<String>().map(String::as_str),
        Some("global")
    );
    assert!(context.prepared().is_none());
    assert!(context.app().is_none());
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
