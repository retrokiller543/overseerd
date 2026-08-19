//! Typed generated CLI command implementations.

mod database_migrate;
mod inspect_registry;

pub use database_migrate::DatabaseMigrateCommand;
pub use inspect_registry::InspectRegistryCommand;

/// Shared arguments available before or after every generated subcommand.
#[derive(clap::Args)]
pub struct OutputArgs {
    /// Print detailed registry and lifecycle information.
    #[arg(long, global = true)]
    pub(crate) verbose: bool,
}
