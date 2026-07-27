//! Typed generated CLI command implementations.

mod check_database;
mod inspect_registry;

pub use check_database::CheckDatabaseCommand;
pub use inspect_registry::InspectRegistryCommand;

/// Shared arguments available before or after every generated subcommand.
#[derive(clap::Args)]
pub struct OutputArgs {
    /// Print additional command details.
    #[arg(long, global = true)]
    pub(crate) verbose: bool,
}
