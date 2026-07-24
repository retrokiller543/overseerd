use std::collections::HashSet;

/// A structural conflict in a generated Clap command definition.
#[derive(Debug, thiserror::Error)]
#[error("invalid command-line definition at `{command}`: duplicate {kind} `{value}`")]
pub struct CliDefinitionError {
    command: String,
    kind: &'static str,
    value: String,
}

/// Validates generated and flattened Clap declarations before Clap builds the parser.
#[doc(hidden)]
pub fn validate_cli(command: &clap::Command) -> Result<(), CliDefinitionError> {
    validate_command(command, &[], &[], &[], &[])
}

fn validate_command(
    command: &clap::Command,
    inherited_ids: &[&str],
    inherited_longs: &[&str],
    inherited_shorts: &[char],
    parent_path: &[&str],
) -> Result<(), CliDefinitionError> {
    let mut ids = HashSet::new();
    let mut longs = HashSet::new();
    let mut shorts = HashSet::new();
    let mut global_ids = inherited_ids.to_vec();
    let mut global_longs = inherited_longs.to_vec();
    let mut global_shorts = inherited_shorts.to_vec();
    let mut path = parent_path.to_vec();
    let mut subcommand_names = HashSet::new();

    path.push(command.get_name());

    if !command.is_disable_help_flag_set() {
        insert_unique(&mut ids, inherited_ids, "help", "argument id", &path)?;
        insert_unique(&mut longs, inherited_longs, "help", "long option", &path)?;
        insert_unique(&mut shorts, inherited_shorts, 'h', "short option", &path)?;
    }

    if command.get_version().is_some() && !command.is_disable_version_flag_set() {
        insert_unique(&mut ids, inherited_ids, "version", "argument id", &path)?;
        insert_unique(&mut longs, inherited_longs, "version", "long option", &path)?;
        insert_unique(&mut shorts, inherited_shorts, 'V', "short option", &path)?;
    }

    if !command.is_disable_help_subcommand_set() && command.has_subcommands() {
        subcommand_names.insert("help");
    }

    for argument in command.get_arguments() {
        let id = argument.get_id().as_str();

        insert_unique(&mut ids, inherited_ids, id, "argument id", &path)?;

        if argument.is_global_set() {
            global_ids.push(id);
        }

        if let Some(long) = argument.get_long() {
            insert_unique(&mut longs, inherited_longs, long, "long option", &path)?;

            if argument.is_global_set() {
                global_longs.push(long);
            }
        }

        for alias in argument.get_all_aliases().into_iter().flatten() {
            insert_unique(&mut longs, inherited_longs, alias, "long option", &path)?;

            if argument.is_global_set() {
                global_longs.push(alias);
            }
        }

        if let Some(short) = argument.get_short() {
            insert_unique(&mut shorts, inherited_shorts, short, "short option", &path)?;

            if argument.is_global_set() {
                global_shorts.push(short);
            }
        }

        for alias in argument.get_all_short_aliases().into_iter().flatten() {
            insert_unique(&mut shorts, inherited_shorts, alias, "short option", &path)?;

            if argument.is_global_set() {
                global_shorts.push(alias);
            }
        }
    }

    for group in command.get_groups() {
        insert_unique(
            &mut ids,
            inherited_ids,
            group.get_id().as_str(),
            "argument or group id",
            &path,
        )?;
    }

    for subcommand in command.get_subcommands() {
        insert_unique(
            &mut subcommand_names,
            &[],
            subcommand.get_name(),
            "subcommand name or alias",
            &path,
        )?;

        for alias in subcommand.get_all_aliases() {
            insert_unique(
                &mut subcommand_names,
                &[],
                alias,
                "subcommand name or alias",
                &path,
            )?;
        }

        if let Some(long) = subcommand.get_long_flag() {
            insert_unique(&mut longs, inherited_longs, long, "long option", &path)?;
        }

        for alias in subcommand.get_all_long_flag_aliases() {
            insert_unique(&mut longs, inherited_longs, alias, "long option", &path)?;
        }

        if let Some(short) = subcommand.get_short_flag() {
            insert_unique(&mut shorts, inherited_shorts, short, "short option", &path)?;
        }

        for alias in subcommand.get_all_short_flag_aliases() {
            insert_unique(&mut shorts, inherited_shorts, alias, "short option", &path)?;
        }

        validate_command(
            subcommand,
            &global_ids,
            &global_longs,
            &global_shorts,
            &path,
        )?;
    }

    Ok(())
}

fn insert_unique<T>(
    local: &mut HashSet<T>,
    inherited: &[T],
    value: T,
    kind: &'static str,
    path: &[&str],
) -> Result<(), CliDefinitionError>
where
    T: Copy + Eq + std::hash::Hash + ToString,
{
    if inherited.contains(&value) || !local.insert(value) {
        return Err(CliDefinitionError {
            command: path.join(" "),
            kind,
            value: value.to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests;
