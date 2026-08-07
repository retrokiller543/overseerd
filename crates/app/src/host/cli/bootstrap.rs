use std::io::IsTerminal as _;

use upwell_config::{ConfigManager, Dynamic};
use upwell_dirs::DirectoriesManager;

use super::{BootstrapError, BootstrapOptions, BootstrapPolicy, BootstrapState, ColorChoice};
use crate::{
    AppBuilder, BootstrapContext, ExecutionMode, LogFormat, LoggingConfig, ProtocolDefinition,
};

#[derive(Default)]
pub(super) struct BootstrapEnvironment {
    pub(super) config: Option<std::ffi::OsString>,
    pub(super) profiles: Option<String>,
    pub(super) rust_log: Option<String>,
    pub(super) log_format: Option<String>,
    pub(super) no_color: bool,
    pub(super) color_force: Option<String>,
    pub(super) stdout_terminal: bool,
}

#[derive(Clone, Copy, Default)]
struct LoggingSourcePresence {
    level: bool,
    format: bool,
    ansi: bool,
}

impl BootstrapEnvironment {
    fn capture() -> Self {
        Self {
            config: std::env::var_os("UPWELL_CONFIG"),
            profiles: std::env::var("UPWELL_PROFILES").ok(),
            rust_log: std::env::var("RUST_LOG").ok(),
            log_format: std::env::var("UPWELL_LOG_FORMAT").ok(),
            no_color: std::env::var_os("NO_COLOR").is_some(),
            color_force: std::env::var("CLICOLOR_FORCE").ok(),
            stdout_terminal: std::io::stdout().is_terminal(),
        }
    }
}

/// Resolves generated application bootstrap without parsing process arguments.
pub fn bootstrap_application(
    application: &str,
    mode: ExecutionMode,
    options: BootstrapOptions,
) -> Result<BootstrapContext, BootstrapError> {
    bootstrap_application_with_policy(application, mode, options, BootstrapPolicy::default())
}

/// Resolves generated bootstrap with explicit declaration-manager ownership.
pub fn bootstrap_application_with_policy(
    application: &str,
    mode: ExecutionMode,
    options: BootstrapOptions,
    policy: BootstrapPolicy,
) -> Result<BootstrapContext, BootstrapError> {
    bootstrap_application_with_env(
        application,
        mode,
        options,
        policy,
        BootstrapEnvironment::capture(),
    )
}

/// Applies generated bootstrap directories to a protocol-specific application builder.
pub fn configure_bootstrap_directories<D: ProtocolDefinition>(
    context: &mut BootstrapContext,
    builder: AppBuilder<D>,
) -> AppBuilder<D> {
    let Some(state) = context.bootstrap_mut() else {
        return builder;
    };

    match state.directories().cloned() {
        Some(directories) => builder.directories(directories),
        None => builder,
    }
}

/// Applies the generated bootstrap config source to a protocol-specific application builder.
pub fn configure_bootstrap_config<D: ProtocolDefinition>(
    context: &mut BootstrapContext,
    builder: AppBuilder<D>,
) -> AppBuilder<D> {
    let Some(state) = context.bootstrap_mut() else {
        return builder;
    };

    match state.take_config() {
        Some(config) => builder.config_source(config),
        None => builder,
    }
}

/// Installs generated tracing after custom setup has contributed optional layers.
pub fn finalize_bootstrap(context: &mut BootstrapContext) -> Result<(), BootstrapError> {
    let mode = context.mode();
    let Some(state) = context.bootstrap_mut() else {
        return Ok(());
    };

    if !mode.is_run() || state.tracing_installed {
        return Ok(());
    }

    #[cfg(feature = "tracing-subscriber")]
    {
        let layers = std::mem::take(&mut state.tracing_layers);

        crate::builtins::logging::init_tracing_resolved(&state.logging, layers)?;
        state.tracing_installed = true;
    }

    Ok(())
}

pub(super) fn bootstrap_application_with_env(
    application: &str,
    mode: ExecutionMode,
    options: BootstrapOptions,
    policy: BootstrapPolicy,
    environment: BootstrapEnvironment,
) -> Result<BootstrapContext, BootstrapError> {
    let directories = resolve_directories(application, policy)?;
    let command_line_config = options
        .config_is_command_line()
        .then(|| options.config().map(std::path::Path::to_path_buf))
        .flatten();
    let default_config = options
        .config_is_default()
        .then(|| options.config().map(std::path::Path::to_path_buf))
        .flatten();
    let selected_config = command_line_config
        .or_else(|| environment.config.as_ref().map(std::path::PathBuf::from))
        .or(default_config);
    let config_path = selected_config
        .clone()
        .or_else(|| directories.as_ref().map(DirectoriesManager::config_path))
        .unwrap_or_default();
    let profiles = effective_profiles(&options, environment.profiles.as_deref());
    let (config, logging_sources) = load_config(
        &config_path,
        selected_config.is_none(),
        &profiles,
        policy,
        directories.as_ref(),
    )?;
    let mut logging = resolve_logging(&config, logging_sources, &options, &environment)?;
    let color = effective_color(&options, &environment, &config, logging_sources)?;

    logging.ansi = match color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => environment.stdout_terminal,
    };

    let state = BootstrapState {
        options,
        config_path,
        profiles,
        directories,
        config,
        logging,
        color,
        tracing_installed: false,
        #[cfg(feature = "tracing-subscriber")]
        tracing_layers: Vec::new(),
    };
    let mut context = BootstrapContext::new(mode);

    context.set_bootstrap(state);

    Ok(context)
}

fn resolve_directories(
    application: &str,
    policy: BootstrapPolicy,
) -> Result<Option<DirectoriesManager>, BootstrapError> {
    if !policy.directories && !policy.config {
        return Ok(None);
    }

    DirectoriesManager::try_for_app(application)
        .map(Some)
        .map_err(BootstrapError::Directories)
}

fn load_config(
    path: &std::path::Path,
    default_path: bool,
    profiles: &[String],
    policy: BootstrapPolicy,
    directories: Option<&DirectoriesManager>,
) -> Result<(Option<ConfigManager<Dynamic>>, LoggingSourcePresence), BootstrapError> {
    if !policy.config {
        return Ok((None, LoggingSourcePresence::default()));
    }

    let config = if path.is_dir() || default_path {
        ConfigManager::<Dynamic>::load_in_explicit(path, profiles)?
    } else if path.is_file() {
        ConfigManager::<Dynamic>::load_file(path, profiles)?
    } else {
        return Err(BootstrapError::MissingConfigPath {
            path: path.to_path_buf(),
        });
    };
    let config = match directories {
        Some(directories) => config.with_directories(directories),
        None => config,
    };
    let logging_sources = LoggingSourcePresence {
        level: config.contains("logging.level"),
        format: config.contains("logging.format"),
        ansi: config.contains("logging.ansi"),
    };

    Ok((
        Some(
            config
                .auto_discover()
                .with_config::<LoggingConfig>("logging"),
        ),
        logging_sources,
    ))
}

fn resolve_logging(
    config: &Option<ConfigManager<Dynamic>>,
    sources: LoggingSourcePresence,
    options: &BootstrapOptions,
    environment: &BootstrapEnvironment,
) -> Result<LoggingConfig, BootstrapError> {
    let mut logging = match config {
        Some(config) => config.get_config::<LoggingConfig>("logging")?,
        None => LoggingConfig::default(),
    };

    if options.log_is_command_line()
        && let Some(filter) = options.log()
    {
        logging.level = filter.to_owned();
    } else if let Some(filter) = environment.rust_log.as_deref() {
        logging.level = filter.to_owned();
    } else if !sources.level
        && options.log_is_default()
        && let Some(filter) = options.log()
    {
        logging.level = filter.to_owned();
    }

    if options.log_format_is_command_line()
        && let Some(format) = options.log_format()
    {
        logging.format = format;
    } else if let Some(format) = environment.log_format.as_deref() {
        logging.format = parse_log_format(format)?;
    } else if !sources.format
        && options.log_format_is_default()
        && let Some(format) = options.log_format()
    {
        logging.format = format;
    }

    Ok(logging)
}

fn effective_profiles(options: &BootstrapOptions, environment: Option<&str>) -> Vec<String> {
    if options.profiles_are_command_line() {
        return options.profiles().to_vec();
    }

    let profiles = environment
        .into_iter()
        .flat_map(|profiles| profiles.split(','))
        .map(str::trim)
        .filter(|profile| !profile.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();

    if !profiles.is_empty() {
        return profiles;
    }

    if options.profiles_are_default() {
        return options.profiles().to_vec();
    }

    Vec::new()
}

fn effective_color(
    options: &BootstrapOptions,
    environment: &BootstrapEnvironment,
    config: &Option<ConfigManager<Dynamic>>,
    sources: LoggingSourcePresence,
) -> Result<ColorChoice, BootstrapError> {
    if options.color_is_command_line()
        && let Some(color) = options.color()
    {
        return Ok(color);
    }

    if environment.no_color {
        return Ok(ColorChoice::Never);
    }

    if environment
        .color_force
        .as_deref()
        .is_some_and(|value| value != "0")
    {
        return Ok(ColorChoice::Always);
    }

    if sources.ansi
        && let Some(config) = config.as_ref()
    {
        let logging = config.get_config::<LoggingConfig>("logging")?;

        return Ok(if logging.ansi {
            ColorChoice::Always
        } else {
            ColorChoice::Never
        });
    }

    if options.color_is_default()
        && let Some(color) = options.color()
    {
        return Ok(color);
    }

    Ok(ColorChoice::Auto)
}

fn parse_log_format(value: &str) -> Result<LogFormat, BootstrapError> {
    match value {
        "full" => Ok(LogFormat::Full),
        "compact" => Ok(LogFormat::Compact),
        "pretty" => Ok(LogFormat::Pretty),
        "json" => Ok(LogFormat::Json),
        _ => Err(BootstrapError::LogFormat {
            value: value.to_owned(),
        }),
    }
}
