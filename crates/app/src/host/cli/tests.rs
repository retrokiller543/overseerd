use super::{
    BootstrapEnvironment, BootstrapOptions, BootstrapPolicy, ColorChoice,
    bootstrap_application_with_env,
};
use crate::{BootstrapContext, ExecutionMode, LogFormat};
use upwell_test_utils::TempFixture;

fn options(config: impl Into<std::path::PathBuf>) -> BootstrapOptions {
    BootstrapOptions::from_parts(
        Some(config.into()),
        Vec::new(),
        None,
        None,
        None,
        [
            Some(clap::parser::ValueSource::CommandLine),
            None,
            None,
            None,
            None,
        ],
    )
}

fn bootstrap(
    application: &str,
    options: BootstrapOptions,
    policy: BootstrapPolicy,
    environment: BootstrapEnvironment,
) -> BootstrapContext {
    bootstrap_application_with_env(
        application,
        ExecutionMode::Tooling,
        options,
        policy,
        environment,
    )
    .expect("bootstrap resolves")
}

#[test]
fn cli_values_override_environment_and_profile_config() {
    let fixture = TempFixture::new("upwell-bootstrap-precedence");
    let config = fixture.write(
        "custom.toml",
        "[logging]\nlevel = \"info\"\nformat = \"full\"\nansi = true\n",
    );

    fixture.write(
        "custom-cli.toml",
        "[logging]\nlevel = \"debug\"\nformat = \"compact\"\n",
    );
    fixture.write("custom-env.toml", "[logging]\nlevel = \"error\"\n");

    let options = BootstrapOptions::from_parts(
        Some(config.clone()),
        vec![String::from("cli")],
        Some(String::from("trace,upwell=debug")),
        Some(LogFormat::Json),
        Some(ColorChoice::Always),
        [Some(clap::parser::ValueSource::CommandLine); 5],
    );
    let environment = BootstrapEnvironment {
        profiles: Some(String::from("env")),
        rust_log: Some(String::from("warn")),
        log_format: Some(String::from("pretty")),
        no_color: true,
        ..BootstrapEnvironment::default()
    };
    let context = bootstrap(
        "bootstrap-precedence-test",
        options,
        BootstrapPolicy::default(),
        environment,
    );
    let state = context.bootstrap().expect("bootstrap state exists");

    assert_eq!(state.config_path(), config);
    assert_eq!(state.profiles(), ["cli"]);
    assert_eq!(state.logging().level, "trace,upwell=debug");
    assert_eq!(state.logging().format, LogFormat::Json);
    assert!(state.logging().ansi);
    assert_eq!(state.color(), ColorChoice::Always);
    assert!(!state.tracing_installed());
}

#[test]
fn environment_is_used_when_cli_values_are_absent() {
    let fixture = TempFixture::new("upwell-bootstrap-environment");
    let config = fixture.write("custom.toml", "");

    fixture.write(
        "custom-env.toml",
        "[logging]\nlevel = \"debug\"\nformat = \"compact\"\n",
    );

    let environment = BootstrapEnvironment {
        config: Some(config.clone().into_os_string()),
        profiles: Some(String::from("env")),
        rust_log: Some(String::from("warn,upwell=trace")),
        log_format: Some(String::from("pretty")),
        no_color: true,
        color_force: Some(String::from("1")),
        stdout_terminal: true,
    };
    let context = bootstrap(
        "bootstrap-environment-test",
        BootstrapOptions::default(),
        BootstrapPolicy::default(),
        environment,
    );
    let state = context.bootstrap().expect("bootstrap state exists");

    assert_eq!(state.profiles(), ["env"]);
    assert_eq!(state.logging().level, "warn,upwell=trace");
    assert_eq!(state.logging().format, LogFormat::Pretty);
    assert!(!state.logging().ansi);
    assert_eq!(state.color(), ColorChoice::Never);
}

#[test]
fn parser_defaults_preserve_environment_and_config_precedence() {
    let fixture = TempFixture::new("upwell-bootstrap-parser-default-precedence");
    let config = fixture.write(
        "application.toml",
        "[logging]\nlevel = \"debug\"\nformat = \"compact\"\nansi = true\n",
    );

    let options = BootstrapOptions::from_parts(
        Some(std::path::PathBuf::from("unused.toml")),
        vec![String::from("application")],
        Some(String::from("error")),
        Some(LogFormat::Json),
        Some(ColorChoice::Never),
        [Some(clap::parser::ValueSource::DefaultValue); 5],
    );
    let environment = BootstrapEnvironment {
        config: Some(config.clone().into_os_string()),
        profiles: Some(String::from("environment")),
        ..BootstrapEnvironment::default()
    };
    let context = bootstrap(
        "bootstrap-parser-default-test",
        options,
        BootstrapPolicy::default(),
        environment,
    );
    let state = context.bootstrap().expect("bootstrap state exists");

    assert_eq!(state.config_path(), config);
    assert_eq!(state.profiles(), ["environment"]);
    assert_eq!(state.logging().level, "debug");
    assert_eq!(state.logging().format, LogFormat::Compact);
    assert_eq!(state.color(), ColorChoice::Always);
}

#[test]
fn parser_defaults_fill_absent_sources() {
    let fixture = TempFixture::new("upwell-bootstrap-parser-default-fallback");
    let config = fixture.write("application.toml", "");

    let options = BootstrapOptions::from_parts(
        Some(config.clone()),
        vec![String::from("application")],
        Some(String::from("error")),
        Some(LogFormat::Json),
        Some(ColorChoice::Never),
        [
            Some(clap::parser::ValueSource::CommandLine),
            Some(clap::parser::ValueSource::DefaultValue),
            Some(clap::parser::ValueSource::DefaultValue),
            Some(clap::parser::ValueSource::DefaultValue),
            Some(clap::parser::ValueSource::DefaultValue),
        ],
    );
    let context = bootstrap(
        "bootstrap-parser-default-fallback-test",
        options,
        BootstrapPolicy::default(),
        BootstrapEnvironment::default(),
    );
    let state = context.bootstrap().expect("bootstrap state exists");

    assert_eq!(state.config_path(), config);
    assert_eq!(state.profiles(), ["application"]);
    assert_eq!(state.logging().level, "error");
    assert_eq!(state.logging().format, LogFormat::Json);
    assert_eq!(state.color(), ColorChoice::Never);
}

#[test]
fn auto_color_follows_terminal_capability() {
    let fixture = TempFixture::new("upwell-bootstrap-terminal");
    let config = fixture.write("application.toml", "");

    for (terminal, ansi) in [(false, false), (true, true)] {
        let options = BootstrapOptions::from_parts(
            Some(config.clone()),
            Vec::new(),
            None,
            None,
            Some(ColorChoice::Auto),
            [
                Some(clap::parser::ValueSource::CommandLine),
                None,
                None,
                None,
                Some(clap::parser::ValueSource::CommandLine),
            ],
        );

        let environment = BootstrapEnvironment {
            stdout_terminal: terminal,
            ..BootstrapEnvironment::default()
        };
        let context = bootstrap(
            "bootstrap-terminal-test",
            options,
            BootstrapPolicy::default(),
            environment,
        );

        assert_eq!(
            context
                .bootstrap()
                .expect("bootstrap state exists")
                .logging()
                .ansi,
            ansi
        );
    }
}

#[test]
fn existing_dotted_path_is_treated_as_directory() {
    let fixture = TempFixture::new("upwell-bootstrap-directory.d");

    fixture.write("application.toml", "");

    let context = bootstrap(
        "bootstrap-dotted-directory-test",
        options(fixture.path()),
        BootstrapPolicy::default(),
        BootstrapEnvironment::default(),
    );

    assert_eq!(
        context
            .bootstrap()
            .expect("bootstrap state exists")
            .config_path(),
        fixture.path()
    );
}

#[test]
fn missing_explicit_config_path_is_rejected() {
    let fixture = TempFixture::new("upwell-bootstrap-missing");
    let path = fixture.child("missing.toml");
    let result = bootstrap_application_with_env(
        "bootstrap-missing-path-test",
        ExecutionMode::Tooling,
        options(path.clone()),
        BootstrapPolicy::default(),
        BootstrapEnvironment::default(),
    );
    let error = match result {
        Ok(_) => panic!("missing explicit config path was accepted"),
        Err(error) => error,
    };

    assert!(
        matches!(error, super::BootstrapError::MissingConfigPath { path: error_path } if error_path == path)
    );
}

#[test]
fn declaration_owned_config_skips_generated_loading() {
    let fixture = TempFixture::new("upwell-bootstrap-ignored");
    let path = fixture.child("ignored.toml");
    let options = BootstrapOptions::from_parts(
        Some(path.clone()),
        Vec::new(),
        Some(String::from("debug")),
        None,
        None,
        [
            Some(clap::parser::ValueSource::CommandLine),
            None,
            Some(clap::parser::ValueSource::CommandLine),
            None,
            None,
        ],
    );

    let context = bootstrap(
        "bootstrap-declaration-config-test",
        options,
        BootstrapPolicy::new(false, false),
        BootstrapEnvironment::default(),
    );
    let state = context.bootstrap().expect("bootstrap state exists");

    assert_eq!(state.config_path(), path);
    assert!(state.directories().is_none());
    assert_eq!(state.logging().level, "debug");
}

#[test]
fn default_options_have_no_generated_values_or_sources() {
    let options = BootstrapOptions::default();

    assert!(options.config().is_none());
    assert!(options.profiles().is_empty());
    assert!(options.log().is_none());
    assert!(options.log_format().is_none());
    assert!(options.color().is_none());
    assert!(!options.config_is_command_line());
    assert!(!options.config_is_default());
    assert!(!options.profiles_are_command_line());
    assert!(!options.profiles_are_default());
    assert!(!options.log_is_command_line());
    assert!(!options.log_is_default());
    assert!(!options.log_format_is_command_line());
    assert!(!options.log_format_is_default());
    assert!(!options.color_is_command_line());
    assert!(!options.color_is_default());
}
