use std::any::TypeId;

use clap::{Arg, Command};

use super::PluginCliProvider;
use crate::{CliDefinitionError, CliDefinitionSource, validate_cli};

#[cfg(feature = "tooling")]
use crate::host::cli::metadata::CliOwnership;

pub(crate) struct AugmentedCli {
    pub(crate) command: Command,
    #[cfg(feature = "tooling")]
    pub(crate) metadata: upwell_tooling_schema::CliMetadata,
}

pub(crate) fn augment_plugin_cli(
    base: Command,
    framework: Command,
    providers: &[PluginCliProvider],
    application_args: &[TypeId],
    serve_default: bool,
) -> Result<AugmentedCli, CliDefinitionError> {
    if let Err(error) = validate_cli(&base) {
        let framework_collision = validate_cli(&framework)
            .is_err_and(|framework_error| framework_error.same_collision(&error));
        let framework_owned = framework_owns_collision(&base, &framework, &error);
        let first = if framework_owned {
            CliDefinitionSource::Framework
        } else {
            CliDefinitionSource::Application
        };
        let second = if framework_collision {
            CliDefinitionSource::Framework
        } else {
            CliDefinitionSource::Application
        };

        return Err(error.with_sources(first, second));
    }

    validate_arg_types(base.get_name(), providers, application_args)?;

    let mut command = base.clone();
    let mut applied = Vec::new();
    #[cfg(feature = "tooling")]
    let mut ownership = CliOwnership::application(&base, &framework);

    for provider in providers {
        let candidate = provider.augment(command.clone());

        if let Err(error) = validate_cli(&candidate) {
            let first = find_prior_source(&base, &framework, &applied, provider, &error);
            let second = CliDefinitionSource::Plugin(provider.metadata().provenance());

            return Err(error.with_sources(first, second));
        }

        #[cfg(feature = "tooling")]
        ownership.capture_provider(&command, &candidate, provider.metadata().provenance());
        command = candidate;
        applied.push(provider);
    }

    #[cfg(feature = "tooling")]
    let metadata = ownership.extract(
        &command,
        &providers
            .iter()
            .map(|provider| provider.metadata())
            .collect::<Vec<_>>(),
        serve_default,
    );
    #[cfg(not(feature = "tooling"))]
    let _ = serve_default;

    Ok(AugmentedCli {
        command,
        #[cfg(feature = "tooling")]
        metadata,
    })
}

fn validate_arg_types(
    command: &str,
    providers: &[PluginCliProvider],
    application_args: &[TypeId],
) -> Result<(), CliDefinitionError> {
    let mut types: Vec<(TypeId, CliDefinitionSource)> = application_args
        .iter()
        .copied()
        .map(|type_id| (type_id, CliDefinitionSource::Application))
        .collect();

    for provider in providers {
        let PluginCliProvider::Args {
            metadata,
            value_type,
            ..
        } = provider
        else {
            continue;
        };
        let source = CliDefinitionSource::Plugin(metadata.provenance());

        if let Some((_, first)) = types.iter().find(|(type_id, _)| type_id == value_type) {
            return Err(CliDefinitionError::duplicate(
                command,
                "typed global argument provider",
                "TypeId",
                *first,
                source,
            ));
        }

        types.push((*value_type, source));
    }

    Ok(())
}

fn find_prior_source(
    base: &Command,
    framework: &Command,
    applied: &[&PluginCliProvider],
    current: &PluginCliProvider,
    collision: &CliDefinitionError,
) -> CliDefinitionSource {
    let provider_source = CliDefinitionSource::Plugin(current.metadata().provenance());
    let provider_only = current.augment(
        Command::new("plugin")
            .disable_help_flag(true)
            .disable_help_subcommand(true)
            .disable_version_flag(true),
    );

    if validate_cli(&provider_only).is_err_and(|error| error.same_collision(collision)) {
        return provider_source;
    }

    let with_current = current.augment(base.clone());

    if validate_cli(&with_current).is_err_and(|error| error.same_collision(collision)) {
        if framework_owns_collision(&with_current, framework, collision) {
            return CliDefinitionSource::Framework;
        }

        return CliDefinitionSource::Application;
    }

    for prior in applied {
        let with_prior = prior.augment(base.clone());
        let with_both = current.augment(with_prior);

        if validate_cli(&with_both).is_err_and(|error| error.same_collision(collision)) {
            return CliDefinitionSource::Plugin(prior.metadata().provenance());
        }
    }

    CliDefinitionSource::Application
}

fn framework_owns_collision(
    full: &Command,
    framework: &Command,
    collision: &CliDefinitionError,
) -> bool {
    let mut framework = framework.clone();

    framework.build();

    let collision_path = collision.command().split(' ').collect::<Vec<_>>();
    let relative_path = collision_path.get(1..).unwrap_or_default();
    let target = command_at_path(full, relative_path);
    let framework_target = command_at_path(&framework, relative_path);
    let root_globals = framework
        .get_arguments()
        .filter(|argument| argument.is_global_set());
    let inherited_match = root_globals
        .into_iter()
        .any(|argument| argument_claims(argument, collision));
    let local_argument_match = framework_target.is_some_and(|command| {
        command
            .get_arguments()
            .any(|argument| argument_claims(argument, collision))
    });
    let local_command_match = framework_target.is_some_and(|command| {
        command
            .get_subcommands()
            .any(|child| command_claims(child, collision))
    });
    let generated_match = target.is_some_and(|command| generated_claims(command, collision));

    inherited_match || local_argument_match || local_command_match || generated_match
}

fn command_at_path<'a>(command: &'a Command, path: &[&str]) -> Option<&'a Command> {
    let mut command = command;

    for name in path {
        command = command.get_subcommands().find(|child| {
            child.get_name() == *name || child.get_all_aliases().any(|alias| alias == *name)
        })?;
    }

    Some(command)
}

fn argument_claims(argument: &Arg, collision: &CliDefinitionError) -> bool {
    match collision.kind() {
        "argument id" | "argument or group id" => argument.get_id().as_str() == collision.value(),
        "long option" => {
            argument.get_long() == Some(collision.value())
                || argument
                    .get_all_aliases()
                    .into_iter()
                    .flatten()
                    .any(|alias| alias == collision.value())
        }
        "short option" => collision.value().chars().next().is_some_and(|value| {
            argument.get_short() == Some(value)
                || argument
                    .get_all_short_aliases()
                    .into_iter()
                    .flatten()
                    .any(|alias| alias == value)
        }),
        _ => false,
    }
}

fn command_claims(command: &Command, collision: &CliDefinitionError) -> bool {
    match collision.kind() {
        "subcommand name or alias" => {
            command.get_name() == collision.value()
                || command
                    .get_all_aliases()
                    .any(|alias| alias == collision.value())
        }
        "long option" => {
            command.get_long_flag() == Some(collision.value())
                || command
                    .get_all_long_flag_aliases()
                    .any(|alias| alias == collision.value())
        }
        "short option" => collision.value().chars().next().is_some_and(|value| {
            command.get_short_flag() == Some(value)
                || command
                    .get_all_short_flag_aliases()
                    .any(|alias| alias == value)
        }),
        _ => false,
    }
}

fn generated_claims(command: &Command, collision: &CliDefinitionError) -> bool {
    let help = !command.is_disable_help_flag_set();
    let version = command.get_version().is_some() && !command.is_disable_version_flag_set();
    let help_command = !command.is_disable_help_subcommand_set() && command.has_subcommands();

    match (collision.kind(), collision.value()) {
        ("argument id" | "argument or group id" | "long option", "help") => help,
        ("short option", "h") => help,
        ("argument id" | "argument or group id" | "long option", "version") => version,
        ("short option", "V") => version,
        ("subcommand name or alias", "help") => help_command,
        _ => false,
    }
}

#[cfg(test)]
mod tests;
