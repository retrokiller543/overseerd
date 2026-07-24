use std::error::Error as _;

use super::{BootstrapContext, ExecutionMode, LifecyclePhase, PhaseError};

#[cfg(feature = "cli")]
use super::{
    BootstrapEnvironment, BootstrapOptions, BootstrapPolicy, ColorChoice,
    bootstrap_application_with_env,
};

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

#[cfg(feature = "cli")]
#[test]
fn bootstrap_cli_values_override_environment_and_profile_config() {
    let directory = std::env::temp_dir().join(format!(
        "overseerd-bootstrap-precedence-{}",
        std::process::id()
    ));
    let config = directory.join("custom.toml");

    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create config directory");
    std::fs::write(
        &config,
        r#"
            [logging]
            level = "info"
            format = "full"
            ansi = true
        "#,
    )
    .expect("write base config");
    std::fs::write(
        directory.join("custom-cli.toml"),
        r#"
            [logging]
            level = "debug"
            format = "compact"
        "#,
    )
    .expect("write CLI profile");
    std::fs::write(
        directory.join("custom-env.toml"),
        r#"
            [logging]
            level = "error"
        "#,
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
        config: None,
        profiles: Some(String::from("env")),
        rust_log: Some(String::from("warn")),
        log_format: Some(String::from("pretty")),
        no_color: true,
        color_force: None,
        stdout_terminal: false,
    };

    let context = bootstrap_application_with_env(
        "bootstrap-precedence-test",
        ExecutionMode::Tooling,
        options,
        BootstrapPolicy::default(),
        environment,
    )
    .expect("bootstrap resolves");
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

#[cfg(feature = "cli")]
#[test]
fn bootstrap_uses_environment_when_cli_values_are_absent() {
    let directory = std::env::temp_dir().join(format!(
        "overseerd-bootstrap-environment-{}",
        std::process::id()
    ));
    let config = directory.join("custom.toml");

    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create config directory");
    std::fs::write(&config, "").expect("write base config");
    std::fs::write(
        directory.join("custom-env.toml"),
        r#"
            [logging]
            level = "debug"
            format = "compact"
        "#,
    )
    .expect("write environment profile");

    let options = BootstrapOptions {
        config: None,
        profiles: Vec::new(),
        log: None,
        log_format: None,
        color: None,
    };
    let environment = BootstrapEnvironment {
        config: Some(config.clone().into_os_string()),
        profiles: Some(String::from("env")),
        rust_log: Some(String::from("warn,overseerd=trace")),
        log_format: Some(String::from("pretty")),
        no_color: true,
        color_force: Some(String::from("1")),
        stdout_terminal: true,
    };

    let context = bootstrap_application_with_env(
        "bootstrap-environment-test",
        ExecutionMode::Tooling,
        options,
        BootstrapPolicy::default(),
        environment,
    )
    .expect("bootstrap resolves");
    let state = context.bootstrap().expect("bootstrap state exists");

    assert_eq!(state.profiles(), ["env"]);
    assert_eq!(state.logging().level, "warn,overseerd=trace");
    assert_eq!(state.logging().format, LogFormat::Pretty);
    assert!(!state.logging().ansi);
    assert_eq!(state.color(), ColorChoice::Never);

    std::fs::remove_dir_all(directory).expect("remove config directory");
}

#[cfg(feature = "cli")]
#[test]
fn bootstrap_auto_color_follows_terminal_capability() {
    let directory = std::env::temp_dir().join(format!(
        "overseerd-bootstrap-terminal-{}",
        std::process::id()
    ));
    let config = directory.join("application.toml");

    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create config directory");
    std::fs::write(&config, "").expect("write base config");

    for (terminal, ansi) in [(false, false), (true, true)] {
        let options = BootstrapOptions {
            config: Some(config.clone()),
            profiles: Vec::new(),
            log: None,
            log_format: None,
            color: Some(ColorChoice::Auto),
        };
        let environment = BootstrapEnvironment {
            stdout_terminal: terminal,
            ..BootstrapEnvironment::default()
        };
        let context = bootstrap_application_with_env(
            "bootstrap-terminal-test",
            ExecutionMode::Tooling,
            options,
            BootstrapPolicy::default(),
            environment,
        )
        .expect("bootstrap resolves");

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

#[cfg(feature = "cli")]
#[test]
fn bootstrap_treats_existing_dotted_path_as_directory() {
    let directory = std::env::temp_dir().join(format!(
        "overseerd-bootstrap-directory-{}.d",
        std::process::id()
    ));

    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create dotted config directory");
    std::fs::write(directory.join("application.toml"), "").expect("write base config");

    let options = BootstrapOptions {
        config: Some(directory.clone()),
        profiles: Vec::new(),
        log: None,
        log_format: None,
        color: None,
    };
    let context = bootstrap_application_with_env(
        "bootstrap-dotted-directory-test",
        ExecutionMode::Tooling,
        options,
        BootstrapPolicy::default(),
        BootstrapEnvironment::default(),
    )
    .expect("dotted directory loads");

    assert_eq!(
        context
            .bootstrap()
            .expect("bootstrap state exists")
            .config_path(),
        directory
    );

    std::fs::remove_dir_all(directory).expect("remove config directory");
}

#[cfg(feature = "cli")]
#[test]
fn bootstrap_rejects_missing_explicit_config_path() {
    let path = std::env::temp_dir().join(format!(
        "overseerd-bootstrap-missing-{}.toml",
        std::process::id()
    ));
    let options = BootstrapOptions {
        config: Some(path.clone()),
        profiles: Vec::new(),
        log: None,
        log_format: None,
        color: None,
    };
    let result = bootstrap_application_with_env(
        "bootstrap-missing-path-test",
        ExecutionMode::Tooling,
        options,
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

#[cfg(feature = "cli")]
#[test]
fn declaration_owned_config_skips_generated_config_loading() {
    let path = std::env::temp_dir().join(format!(
        "overseerd-bootstrap-ignored-{}.toml",
        std::process::id()
    ));
    let options = BootstrapOptions {
        config: Some(path.clone()),
        profiles: Vec::new(),
        log: Some(String::from("debug")),
        log_format: None,
        color: None,
    };
    let context = bootstrap_application_with_env(
        "bootstrap-declaration-config-test",
        ExecutionMode::Tooling,
        options,
        BootstrapPolicy::new(false, false),
        BootstrapEnvironment::default(),
    )
    .expect("declaration-owned config bypasses generated loading");
    let state = context.bootstrap().expect("bootstrap state exists");

    assert_eq!(state.config_path(), path);
    assert!(state.directories().is_none());
    assert_eq!(state.logging().level, "debug");
}
