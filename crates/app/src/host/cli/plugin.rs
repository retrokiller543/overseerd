use std::any::{Any, TypeId};
use std::future::Future;
use std::pin::Pin;

use clap::{Args, Command, Subcommand};

mod context;
mod validation;

pub(crate) use context::PluginCommandState;
pub use context::{PluginCliCommand, PluginCommandContext};
pub(crate) use validation::augment_plugin_cli;

use super::{CommandContext, CommandError, CommandPhase};
use crate::{BootstrapContext, ContributionId, ContributionProvenance, Contributor, PluginId};

type ParsedValue = Box<dyn Any + Send + Sync>;
type BoxedCommandError = Box<dyn std::error::Error + Send + Sync>;
type CommandFuture<'a> = Pin<Box<dyn Future<Output = Result<(), BoxedCommandError>> + Send + 'a>>;

/// The parser-facing category of an optional plugin CLI contribution.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum PluginCliProviderKind {
    /// A flattened global `clap::Args` group.
    Args,
    /// One named leaf command containing typed `clap::Args`.
    Command,
    /// A flattened native `clap::Subcommand` set.
    CommandSet,
}

/// Deterministic read-only metadata for one effective plugin CLI provider.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PluginCliProviderMetadata {
    provenance: ContributionProvenance,
    kind: PluginCliProviderKind,
}

impl PluginCliProviderMetadata {
    /// The stable plugin and contributor-local CLI provider identity.
    pub const fn provenance(self) -> ContributionProvenance {
        self.provenance
    }

    /// The parser-facing provider category.
    pub const fn kind(self) -> PluginCliProviderKind {
        self.kind
    }
}

/// Collects optional typed CLI facets from one effective plugin.
pub struct PluginCliRegistrar {
    contributor: PluginId,
    providers: Vec<PluginCliProvider>,
}

impl PluginCliRegistrar {
    pub(crate) const fn new(contributor: PluginId) -> Self {
        Self {
            contributor,
            providers: Vec::new(),
        }
    }

    /// Flattens a typed global argument group into the generated parser.
    pub fn args<T>(&mut self, id: ContributionId)
    where
        T: Args + Send + Sync + 'static,
    {
        self.providers.push(PluginCliProvider::Args {
            metadata: self.metadata(id, PluginCliProviderKind::Args),
            value_type: TypeId::of::<T>(),
            augment: T::augment_args,
            extract: extract_args::<T>,
        });
    }

    /// Adds one named typed leaf command to the generated command namespace.
    pub fn command<T>(&mut self, id: ContributionId, name: &'static str)
    where
        T: Args + PluginCliCommand + Send + Sync + 'static,
    {
        self.providers.push(PluginCliProvider::Command {
            metadata: self.metadata(id, PluginCliProviderKind::Command),
            name,
            augment: augment_command::<T>,
            extract: extract_command::<T>,
        });
    }

    /// Flattens a typed native subcommand set into the generated command namespace.
    pub fn commands<T>(&mut self, id: ContributionId)
    where
        T: Subcommand + PluginCliCommand + Send + Sync + 'static,
    {
        self.providers.push(PluginCliProvider::CommandSet {
            metadata: self.metadata(id, PluginCliProviderKind::CommandSet),
            augment: T::augment_subcommands,
            matches: T::has_subcommand,
            extract: extract_command_set::<T>,
        });
    }

    pub(crate) fn finish(self) -> Vec<PluginCliProvider> {
        self.providers
    }

    fn metadata(
        &self,
        id: ContributionId,
        kind: PluginCliProviderKind,
    ) -> PluginCliProviderMetadata {
        PluginCliProviderMetadata {
            provenance: ContributionProvenance::new(Contributor::Plugin(self.contributor), id),
            kind,
        }
    }
}

pub(crate) enum PluginCliProvider {
    Args {
        metadata: PluginCliProviderMetadata,
        value_type: TypeId,
        augment: fn(Command) -> Command,
        extract: fn(&mut clap::ArgMatches) -> Result<ParsedValue, clap::Error>,
    },
    Command {
        metadata: PluginCliProviderMetadata,
        name: &'static str,
        augment: fn(Command, &'static str) -> Command,
        extract: fn(&mut clap::ArgMatches) -> Result<Box<dyn ErasedPluginCliCommand>, clap::Error>,
    },
    CommandSet {
        metadata: PluginCliProviderMetadata,
        augment: fn(Command) -> Command,
        matches: fn(&str) -> bool,
        extract: fn(&mut clap::ArgMatches) -> Result<Box<dyn ErasedPluginCliCommand>, clap::Error>,
    },
}

impl PluginCliProvider {
    pub(crate) const fn metadata(&self) -> PluginCliProviderMetadata {
        match self {
            Self::Args { metadata, .. }
            | Self::Command { metadata, .. }
            | Self::CommandSet { metadata, .. } => *metadata,
        }
    }

    pub(crate) fn augment(&self, command: Command) -> Command {
        match self {
            Self::Args { augment, .. } | Self::CommandSet { augment, .. } => augment(command),
            Self::Command { name, augment, .. } => augment(command, name),
        }
    }

    pub(crate) fn extract_args(
        &self,
        matches: &mut clap::ArgMatches,
    ) -> Result<Option<(TypeId, ParsedValue)>, clap::Error> {
        match self {
            Self::Args {
                value_type,
                extract,
                ..
            } => extract(matches).map(|value| Some((*value_type, value))),
            Self::Command { .. } | Self::CommandSet { .. } => Ok(None),
        }
    }

    pub(crate) fn matches_command(&self, name: &str) -> bool {
        match self {
            Self::Args { .. } => false,
            Self::Command {
                name: command_name, ..
            } => *command_name == name,
            Self::CommandSet { matches, .. } => matches(name),
        }
    }

    pub(crate) fn extract_command(
        &self,
        matches: &mut clap::ArgMatches,
    ) -> Result<Option<SelectedPluginCliCommand>, clap::Error> {
        let command_name = selected_command_path(matches);
        let command = match self {
            Self::Args { .. } => return Ok(None),
            Self::Command { extract, .. } | Self::CommandSet { extract, .. } => extract(matches)?,
        };

        Ok(Some(SelectedPluginCliCommand {
            command: command_name,
            value: command,
        }))
    }
}

fn selected_command_path(matches: &clap::ArgMatches) -> String {
    let mut names = Vec::new();
    let mut current = matches;

    while let Some((name, nested)) = current.subcommand() {
        names.push(name);
        current = nested;
    }

    names.join(" ")
}

/// Parsed plugin argument groups awaiting insertion into bootstrap state.
#[doc(hidden)]
pub struct ParsedPluginArgs(Vec<(TypeId, ParsedValue)>);

impl ParsedPluginArgs {
    pub(crate) const fn new(values: Vec<(TypeId, ParsedValue)>) -> Self {
        Self(values)
    }

    /// Inserts every parsed typed argument group into bootstrap state.
    pub fn apply(self, context: &mut BootstrapContext) {
        for (type_id, value) in self.0 {
            context.insert_boxed(type_id, value);
        }
    }
}

impl Default for ParsedPluginArgs {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

/// One selected parsed plugin command with narrow erased dispatch.
#[doc(hidden)]
pub struct SelectedPluginCliCommand {
    command: String,
    value: Box<dyn ErasedPluginCliCommand>,
}

impl SelectedPluginCliCommand {
    /// The minimum lifecycle phase requested by the parsed command.
    pub fn phase(&self) -> CommandPhase {
        self.value.phase()
    }

    /// Dispatches the command through the already prepared generated command context.
    pub async fn run<H>(&self, context: CommandContext<H>) -> Result<(), CommandError>
    where
        H: crate::AppHost,
    {
        let context = context.into_plugin();

        self.value
            .run(context)
            .await
            .map_err(|source| CommandError::boxed(self.command.clone(), source))
    }
}

pub(crate) trait ErasedPluginCliCommand: Send + Sync {
    fn phase(&self) -> CommandPhase;

    fn run(&self, context: PluginCommandContext) -> CommandFuture<'_>;
}

impl<T> ErasedPluginCliCommand for T
where
    T: PluginCliCommand + Send + Sync,
{
    fn phase(&self) -> CommandPhase {
        PluginCliCommand::phase(self)
    }

    fn run(&self, context: PluginCommandContext) -> CommandFuture<'_> {
        Box::pin(async move {
            PluginCliCommand::run(self, context)
                .await
                .map_err(|source| Box::new(source) as BoxedCommandError)
        })
    }
}

fn extract_args<T>(matches: &mut clap::ArgMatches) -> Result<ParsedValue, clap::Error>
where
    T: Args + Send + Sync + 'static,
{
    let value = T::from_arg_matches_mut(matches)?;

    Ok(Box::new(value))
}

fn augment_command<T>(command: Command, name: &'static str) -> Command
where
    T: Args,
{
    command.subcommand(T::augment_args(Command::new(name)))
}

fn extract_command<T>(
    matches: &mut clap::ArgMatches,
) -> Result<Box<dyn ErasedPluginCliCommand>, clap::Error>
where
    T: Args + PluginCliCommand + Send + Sync + 'static,
{
    let (_, mut matches) = matches
        .remove_subcommand()
        .expect("selected plugin command has subcommand matches");
    let command = T::from_arg_matches_mut(&mut matches)?;

    Ok(Box::new(command))
}

fn extract_command_set<T>(
    matches: &mut clap::ArgMatches,
) -> Result<Box<dyn ErasedPluginCliCommand>, clap::Error>
where
    T: Subcommand + PluginCliCommand + Send + Sync + 'static,
{
    let command = T::from_arg_matches_mut(matches)?;

    Ok(Box::new(command))
}
