use crate::DaemonApplication;
use crate::components::Db;
use overseerd::{CliCommand, CommandContext, CommandPhase, DiError};

/// Builds the application and verifies that the database component resolves from DI.
#[derive(clap::Args)]
#[group(id = "database-check-mode", required = true, multiple = false)]
pub struct CheckDatabaseCommand {
    /// Verify that the database pool resolves from the root container.
    #[arg(long, group = "database-check-mode")]
    pool: bool,

    /// Verify a connection by recording one example query.
    #[arg(long, group = "database-check-mode")]
    connection: bool,
}

impl CliCommand<DaemonApplication> for CheckDatabaseCommand {
    type Error = DiError;

    fn phase(&self) -> CommandPhase {
        CommandPhase::Built
    }

    async fn run(&self, context: CommandContext<DaemonApplication>) -> Result<(), Self::Error> {
        let database = context.resolve::<Db>().await?;

        println!("database component resolved through application DI");

        if self.connection {
            println!("recorded query #{}", database.record_query());
        }

        let _ = self.pool;

        Ok(())
    }
}
