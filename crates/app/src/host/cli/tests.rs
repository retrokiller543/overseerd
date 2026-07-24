use clap::Parser as _;

use super::{
    BootstrapEnvironment, BootstrapOptions, BootstrapPolicy, ColorChoice,
    bootstrap_application_with_env,
};
use crate::{BootstrapContext, ExecutionMode, LogFormat};

#[derive(clap::Parser)]
struct TestCli {
    #[command(flatten)]
    bootstrap: BootstrapOptions,
}

fn temp_config_dir(test: &str) -> std::path::PathBuf {
    let directory =
        std::env::temp_dir().join(format!("overseerd-bootstrap-{test}-{}", std::process::id()));

    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create config directory");

    directory
}

fn options(config: impl Into<std::path::PathBuf>) -> BootstrapOptions {
    BootstrapOptions {
        config: Some(config.into()),
        profiles: Vec::new(),
        log: None,
        log_format: None,
        color: None,
    }
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
fn options_parse_typed_global_values() {
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

#[test]
fn options_reject_unknown_typed_values() {
    let result = TestCli::try_parse_from(["test", "--log-format", "xml"]);
    let error = match result {
        Ok(_) => panic!("unknown log format was accepted"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
}

#[test]
fn cli_values_override_environment_and_profile_config() {
    let directory = temp_config_dir("precedence");
    let config = directory.join("custom.toml");

    std::fs::write(
        &config,
        "[logging]\nlevel = \"info\"\nformat = \"full\"\nansi = true\n",
    )
    .expect("write base config");
    std::fs::write(
        directory.join("custom-cli.toml"),
        "[logging]\nlevel = \"debug\"\nformat = \"compact\"\n",
    )
    .expect("write CLI profile");
    std::fs::write(
        directory.join("custom-env.toml"),
        "[logging]\nlevel = \"error\"\n",
    )
    .expect("write environment profile");

    let options = BootstrapOptions {
        config: Some(config.clone()),
        profiles: vec![String::from("cli")],
        log: Some(String::from("trace,overseerd=debug")),
        log_format: Some(LogFormat::Json),
        color: Some(ColorChoice::Always),
    };
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
    assert_eq!(state.logging().level, "trace,overseerd=debug");
    assert_eq!(state.logging().format, LogFormat::Json);
    assert!(state.logging().ansi);
    assert_eq!(state.color(), ColorChoice::Always);
    assert!(!state.tracing_installed());

    std::fs::remove_dir_all(directory).expect("remove config directory");
}

#[test]
fn environment_is_used_when_cli_values_are_absent() {
    let directory = temp_config_dir("environment");
    let config = directory.join("custom.toml");

    std::fs::write(&config, "").expect("write base config");
    std::fs::write(
        directory.join("custom-env.toml"),
        "[logging]\nlevel = \"debug\"\nformat = \"compact\"\n",
    )
    .expect("write environment profile");

    let environment = BootstrapEnvironment {
        config: Some(config.clone().into_os_string()),
        profiles: Some(String::from("env")),
        rust_log: Some(String::from("warn,overseerd=trace")),
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
    assert_eq!(state.logging().level, "warn,overseerd=trace");
    assert_eq!(state.logging().format, LogFormat::Pretty);
    assert!(!state.logging().ansi);
    assert_eq!(state.color(), ColorChoice::Never);

    std::fs::remove_dir_all(directory).expect("remove config directory");
}

#[test]
fn auto_color_follows_terminal_capability() {
    let directory = temp_config_dir("terminal");
    let config = directory.join("application.toml");

    std::fs::write(&config, "").expect("write base config");

    for (terminal, ansi) in [(false, false), (true, true)] {
        let mut options = options(config.clone());

        options.color = Some(ColorChoice::Auto);

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

    std::fs::remove_dir_all(directory).expect("remove config directory");
}

#[test]
fn existing_dotted_path_is_treated_as_directory() {
    let directory = temp_config_dir("directory.d");

    std::fs::write(directory.join("application.toml"), "").expect("write base config");

    let context = bootstrap(
        "bootstrap-dotted-directory-test",
        options(directory.clone()),
        BootstrapPolicy::default(),
        BootstrapEnvironment::default(),
    );

    assert_eq!(
        context
            .bootstrap()
            .expect("bootstrap state exists")
            .config_path(),
        directory
    );

    std::fs::remove_dir_all(directory).expect("remove config directory");
}

#[test]
fn missing_explicit_config_path_is_rejected() {
    let path = std::env::temp_dir().join(format!(
        "overseerd-bootstrap-missing-{}.toml",
        std::process::id()
    ));
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
    let path = std::env::temp_dir().join(format!(
        "overseerd-bootstrap-ignored-{}.toml",
        std::process::id()
    ));
    let mut options = options(path.clone());

    options.log = Some(String::from("debug"));

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
