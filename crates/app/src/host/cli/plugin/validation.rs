use std::any::TypeId;

use clap::Command;

use super::PluginCliProvider;
use crate::{CliDefinitionError, CliDefinitionSource, validate_cli};

pub(crate) fn augment_plugin_cli(
    base: Command,
    framework: Command,
    providers: &[PluginCliProvider],
    application_args: &[TypeId],
) -> Result<Command, CliDefinitionError> {
    validate_cli(&base)?;
    validate_arg_types(base.get_name(), providers, application_args)?;

    let mut command = base.clone();
    let mut applied = Vec::new();

    for provider in providers {
        let candidate = provider.augment(command.clone());

        if let Err(error) = validate_cli(&candidate) {
            let first = find_prior_source(&base, &framework, &applied, provider, &error);
            let second = CliDefinitionSource::Plugin(provider.metadata().provenance());

            return Err(error.with_sources(first, second));
        }

        command = candidate;
        applied.push(provider);
    }

    Ok(command)
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
        let with_framework = current.augment(framework.clone());

        if validate_cli(&with_framework).is_err_and(|error| error.same_collision(collision)) {
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
