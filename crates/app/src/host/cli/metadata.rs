use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use clap::{Arg, ArgAction, Command};
use upwell_tooling_schema::{
    CliArgument, CliCardinality, CliCommand, CliMetadata, CliOwner, CliProvider, CliProviderKind,
    cli_provider_id as schema_cli_provider_id,
};

use crate::{
    ContributionProvenance, Contributor, PluginCliProviderKind, PluginCliProviderMetadata,
};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ElementKey {
    Command(Vec<String>),
    Argument(Vec<String>, String),
}

#[derive(Clone, Copy)]
enum ElementOwner {
    Framework,
    Application,
    Plugin(ContributionProvenance),
}

/// Tracks ownership while additive providers compose the executable parser.
pub(crate) struct CliOwnership {
    owners: BTreeMap<ElementKey, ElementOwner>,
    framework_commands: BTreeSet<ElementKey>,
}

impl CliOwnership {
    pub(crate) fn application(command: &Command, framework: &Command) -> Self {
        let declared = definition_keys(command);
        let mut command = command.clone();
        let mut framework = framework.clone();
        let mut owners = BTreeMap::new();

        command.build();
        framework.build();

        let framework = definition_keys(&framework);
        let framework_globals = framework_global_ids(&framework);
        let framework_commands = framework
            .iter()
            .filter(|key| matches!(key, ElementKey::Command(_)))
            .cloned()
            .collect();

        visit_definition(&command, &mut Vec::new(), &mut |key, element| {
            let owner = base_owner(&key, element, &framework, &framework_globals, &declared);

            owners.insert(key, owner);
        });

        Self {
            owners,
            framework_commands,
        }
    }

    pub(crate) fn capture_provider(
        &mut self,
        before: &Command,
        after: &Command,
        provenance: ContributionProvenance,
    ) {
        let before = definition_keys(before);
        let owner = ElementOwner::Plugin(provenance);

        visit_definition(after, &mut Vec::new(), &mut |key, _| {
            if !before.contains(&key) {
                self.owners.insert(key, owner);
            }
        });
    }

    pub(crate) fn extract(
        &self,
        command: &Command,
        providers: &[PluginCliProviderMetadata],
        serve_default: bool,
    ) -> CliMetadata {
        let mut command = command.clone();

        command.build();

        CliMetadata {
            root: extract_command(
                &command,
                &mut Vec::new(),
                &mut BTreeSet::new(),
                &self.owners,
                &self.framework_commands,
            ),
            default_command: serve_default.then(|| String::from("serve")),
            providers: providers.iter().copied().map(cli_provider).collect(),
        }
    }
}

enum DefinitionElement<'a> {
    Command,
    Argument(&'a Arg),
}

fn definition_keys(command: &Command) -> BTreeSet<ElementKey> {
    let mut keys = BTreeSet::new();

    visit_definition(command, &mut Vec::new(), &mut |key, _| {
        keys.insert(key);
    });

    keys
}

fn framework_global_ids(keys: &BTreeSet<ElementKey>) -> BTreeSet<String> {
    keys.iter()
        .filter_map(|key| match key {
            ElementKey::Argument(path, id) if path.len() == 1 => Some(id.clone()),
            ElementKey::Command(_) | ElementKey::Argument(_, _) => None,
        })
        .collect()
}

fn visit_definition<'a>(
    command: &'a Command,
    parent: &mut Vec<String>,
    visit: &mut impl FnMut(ElementKey, DefinitionElement<'a>),
) {
    parent.push(command.get_name().to_string());
    visit(
        ElementKey::Command(parent.clone()),
        DefinitionElement::Command,
    );

    for argument in command.get_arguments() {
        visit(
            ElementKey::Argument(parent.clone(), argument.get_id().as_str().to_string()),
            DefinitionElement::Argument(argument),
        );
    }

    for child in command.get_subcommands() {
        visit_definition(child, parent, visit);
    }

    parent.pop();
}

fn base_owner(
    key: &ElementKey,
    element: DefinitionElement<'_>,
    framework: &BTreeSet<ElementKey>,
    framework_globals: &BTreeSet<String>,
    declared: &BTreeSet<ElementKey>,
) -> ElementOwner {
    match element {
        DefinitionElement::Command => {
            let is_root = matches!(key, ElementKey::Command(path) if path.len() == 1);

            if !is_root && (framework.contains(key) || !declared.contains(key)) {
                ElementOwner::Framework
            } else {
                ElementOwner::Application
            }
        }
        DefinitionElement::Argument(argument) => {
            if framework.contains(key)
                || (argument.is_global_set()
                    && framework_globals.contains(argument.get_id().as_str()))
                || generated_owner(argument).is_some()
            {
                ElementOwner::Framework
            } else {
                ElementOwner::Application
            }
        }
    }
}

fn extract_command(
    command: &Command,
    path: &mut Vec<String>,
    inherited_globals: &mut BTreeSet<String>,
    owners: &BTreeMap<ElementKey, ElementOwner>,
    framework_commands: &BTreeSet<ElementKey>,
) -> CliCommand {
    path.push(command.get_name().to_string());

    let key = ElementKey::Command(path.clone());
    let generated_help = command.get_name() == "help"
        && owners
            .get(&key)
            .is_none_or(|owner| matches!(owner, ElementOwner::Framework));
    let owner = if generated_help {
        ElementOwner::Framework
    } else {
        owners
            .get(&key)
            .copied()
            .unwrap_or(ElementOwner::Application)
    };
    let id = (!generated_help && path.len() > 1 && framework_commands.contains(&key))
        .then(|| String::from("serve"));
    let mut aliases: Vec<_> = command.get_aliases().map(str::to_string).collect();
    let mut visible_aliases: Vec<_> = command.get_visible_aliases().map(str::to_string).collect();
    let visible_short_flag_aliases: Vec<_> = command.get_visible_short_flag_aliases().collect();
    let visible_long_flag_aliases: Vec<_> = command
        .get_visible_long_flag_aliases()
        .map(str::to_string)
        .collect();
    let mut short_flag_aliases: Vec<_> = command
        .get_all_short_flag_aliases()
        .filter(|alias| !visible_short_flag_aliases.contains(alias))
        .collect();
    let mut long_flag_aliases: Vec<_> = command
        .get_all_long_flag_aliases()
        .filter(|alias| {
            !visible_long_flag_aliases
                .iter()
                .any(|visible| visible == alias)
        })
        .map(str::to_string)
        .collect();
    let mut local_globals = inherited_globals.clone();
    let mut arguments = Vec::new();
    let mut commands = Vec::new();

    for argument in command.get_arguments() {
        let argument_id = argument.get_id().as_str();

        if argument.is_global_set() && inherited_globals.contains(argument_id) {
            continue;
        }

        arguments.push(extract_argument(argument, path, owners));

        if argument.is_global_set() {
            local_globals.insert(argument_id.to_string());
        }
    }

    if !generated_help {
        for child in command.get_subcommands() {
            commands.push(extract_command(
                child,
                path,
                &mut local_globals.clone(),
                owners,
                framework_commands,
            ));
        }
    }

    aliases.sort();
    aliases.dedup();
    visible_aliases.sort();
    visible_aliases.dedup();
    short_flag_aliases.sort();
    short_flag_aliases.dedup();
    long_flag_aliases.sort();
    long_flag_aliases.dedup();
    arguments.sort_by(|left, right| {
        (left.index.is_none(), left.index, &left.id).cmp(&(
            right.index.is_none(),
            right.index,
            &right.id,
        ))
    });
    commands.sort_by(|left, right| left.name.cmp(&right.name));

    let metadata = CliCommand {
        id,
        name: command.get_name().to_string(),
        aliases,
        visible_aliases,
        short_flag: command.get_short_flag(),
        short_flag_aliases,
        visible_short_flag_aliases,
        long_flag: command.get_long_flag().map(str::to_string),
        long_flag_aliases,
        visible_long_flag_aliases,
        help: command.get_about().map(ToString::to_string),
        long_help: command.get_long_about().map(ToString::to_string),
        hidden: command.is_hide_set(),
        owner: cli_owner(owner),
        arguments,
        commands,
    };

    path.pop();

    metadata
}

fn extract_argument(
    argument: &Arg,
    path: &[String],
    owners: &BTreeMap<ElementKey, ElementOwner>,
) -> CliArgument {
    let id = argument.get_id().as_str();
    let owner = owners
        .get(&ElementKey::Argument(path.to_vec(), id.to_string()))
        .copied()
        .or_else(|| inherited_global_owner(path, id, owners))
        .or_else(|| generated_owner(argument))
        .unwrap_or(ElementOwner::Application);
    let visible_aliases: Vec<_> = argument
        .get_visible_aliases()
        .unwrap_or_default()
        .into_iter()
        .map(str::to_string)
        .collect();
    let aliases = argument
        .get_aliases()
        .unwrap_or_default()
        .into_iter()
        .map(str::to_string)
        .collect();
    let visible_short_aliases = argument.get_visible_short_aliases().unwrap_or_default();
    let short_aliases = argument
        .get_all_short_aliases()
        .unwrap_or_default()
        .into_iter()
        .filter(|alias| !visible_short_aliases.contains(alias))
        .collect();
    let range = argument.get_num_args().unwrap_or_default();
    let max_values = (range.max_values() != usize::MAX).then_some(range.max_values());
    let repeatable = matches!(argument.get_action(), ArgAction::Append | ArgAction::Count);

    CliArgument {
        id: id.to_string(),
        long: argument.get_long().map(str::to_string),
        short: argument.get_short(),
        index: argument.get_index(),
        aliases,
        visible_aliases,
        short_aliases,
        visible_short_aliases,
        help: argument.get_help().map(ToString::to_string),
        long_help: argument.get_long_help().map(ToString::to_string),
        value_names: argument
            .get_value_names()
            .unwrap_or_default()
            .iter()
            .map(ToString::to_string)
            .collect(),
        default_values: argument
            .get_default_values()
            .iter()
            .map(|value| default_value_string(value))
            .collect(),
        required: argument.is_required_set(),
        global: argument.is_global_set(),
        hidden: argument.is_hide_set(),
        cardinality: CliCardinality {
            min_values: range.min_values(),
            max_values,
            repeatable,
        },
        owner: cli_owner(owner),
    }
}

fn default_value_string(value: &std::ffi::OsStr) -> String {
    let Some(value) = value.to_str() else {
        let mut encoded = String::from("os-bytes:");

        for byte in value.as_encoded_bytes() {
            write!(encoded, "{byte:02x}").expect("writing to a String cannot fail");
        }

        return encoded;
    };

    value.to_string()
}

fn generated_owner(argument: &Arg) -> Option<ElementOwner> {
    match argument.get_action() {
        ArgAction::Help | ArgAction::HelpShort | ArgAction::HelpLong
            if argument.get_id().as_str() == "help" =>
        {
            Some(ElementOwner::Framework)
        }
        ArgAction::Version if argument.get_id().as_str() == "version" => {
            Some(ElementOwner::Framework)
        }
        _ => None,
    }
}

fn inherited_global_owner(
    path: &[String],
    id: &str,
    owners: &BTreeMap<ElementKey, ElementOwner>,
) -> Option<ElementOwner> {
    for length in (1..path.len()).rev() {
        let key = ElementKey::Argument(path[..length].to_vec(), id.to_string());

        if let Some(owner) = owners.get(&key) {
            return Some(*owner);
        }
    }

    None
}

fn cli_owner(owner: ElementOwner) -> CliOwner {
    match owner {
        ElementOwner::Framework => CliOwner::Framework,
        ElementOwner::Application => CliOwner::Application,
        ElementOwner::Plugin(provenance) => CliOwner::Plugin {
            provider: cli_provider_id(provenance),
        },
    }
}

fn cli_provider(provider: PluginCliProviderMetadata) -> CliProvider {
    let provenance = provider.provenance();

    CliProvider {
        id: cli_provider_id(provenance),
        contributor: contributor_id(provenance.contributor()),
        contribution: provenance.contribution().as_str().to_string(),
        kind: match provider.kind() {
            PluginCliProviderKind::Args => CliProviderKind::Args,
            PluginCliProviderKind::Command => CliProviderKind::Command,
            PluginCliProviderKind::CommandSet => CliProviderKind::CommandSet,
        },
    }
}

fn cli_provider_id(provenance: ContributionProvenance) -> String {
    schema_cli_provider_id(
        &contributor_id(provenance.contributor()),
        provenance.contribution().as_str(),
    )
}

fn contributor_id(contributor: Contributor) -> String {
    match contributor {
        Contributor::Framework => String::from("framework"),
        Contributor::Application => String::from("application"),
        Contributor::Protocol(protocol) => format!("protocol:{}", protocol.as_str()),
        Contributor::Plugin(plugin) => format!("plugin:{}", plugin.as_str()),
    }
}
