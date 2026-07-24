//! A complete generated Overseerd application host with nested typed CLI commands.
//!
//! The named `app!` declaration below generates the Clap parser, nested subcommand enums,
//! lifecycle dispatcher, and `DaemonApplication::run()` process entry point. Inspect its expansion
//! with `cargo expand -p overseerd-example-daemon --bin overseerd-example-daemon`.
//!
//! ```text
//! cargo run -p overseerd-example-daemon -- --help
//! cargo run -p overseerd-example-daemon -- --config examples/daemon/config/application.toml inspect registry
//! cargo run -p overseerd-example-daemon -- --config examples/daemon/config/application.toml database check --connection
//! cargo run -p overseerd-example-daemon -- --config examples/daemon/config/application.toml
//! ```
//!
//! With no subcommand, the generated CLI selects `serve` and runs until Ctrl-C. The explicit
//! `--config` paths above are for development from the workspace; normal installations load from
//! the platform-native project config directory. Custom runtimes can override directories through
//! an explicitly supplied `DirectoriesManager`.
//!
//! The same host is also a compile-time lifecycle state machine:
//!
//! ```ignore
//! let setup = DaemonApplication::new(ExecutionMode::Run).setup().await?;
//! let prepared = setup.prepare().await?;
//! let built = prepared.build().await?;
//! built.serve().await?;
//!
//! // Explicit fast-forward: still executes setup, prepare, and build in order.
//! DaemonApplication::new(ExecutionMode::Run).serve().await?;
//! ```

mod components;
mod notifiers;
mod service;

use crate::components::{Db, DbConfig};
use overseerd::{
    Cfg, CliCommand, CommandContext, CommandPhase, LoggingConfig, ServerConfig, TcpTransport, app,
};

/// Shared arguments available before or after every generated subcommand.
#[derive(clap::Args)]
struct OutputArgs {
    /// Print additional command details.
    #[arg(long, global = true)]
    verbose: bool,
}

/// Prints the validated registry without constructing components or the RPC protocol.
#[derive(clap::Args)]
struct InspectRegistryCommand;

impl CliCommand<DaemonApplication> for InspectRegistryCommand {
    type Error = std::io::Error;

    fn phase(&self) -> CommandPhase {
        CommandPhase::Configured
    }

    async fn run(&self, context: CommandContext<DaemonApplication>) -> Result<(), Self::Error> {
        let prepared = context
            .prepared()
            .ok_or_else(|| std::io::Error::other("prepared application is unavailable"))?;
        let verbose = context
            .bootstrap()
            .get::<OutputArgs>()
            .is_some_and(|args| args.verbose);

        println!("Application: {}", prepared.name());
        println!("{}", prepared.registry());

        if verbose {
            println!(
                "Protocol: {}",
                std::any::type_name_of_val(prepared.protocol())
            );
        }

        Ok(())
    }
}

/// Builds the application and verifies that the database component resolves from DI.
#[derive(clap::Args)]
#[group(id = "database-check-mode", required = true, multiple = false)]
struct CheckDatabaseCommand {
    /// Verify that the database pool resolves from the root container.
    #[arg(long, group = "database-check-mode")]
    pool: bool,

    /// Verify a connection by recording one example query.
    #[arg(long, group = "database-check-mode")]
    connection: bool,
}

impl CliCommand<DaemonApplication> for CheckDatabaseCommand {
    type Error = std::io::Error;

    fn phase(&self) -> CommandPhase {
        CommandPhase::Built
    }

    async fn run(&self, context: CommandContext<DaemonApplication>) -> Result<(), Self::Error> {
        let app = context
            .app()
            .ok_or_else(|| std::io::Error::other("built application is unavailable"))?;
        let database = app
            .container()
            .get::<Db>()
            .ok_or_else(|| std::io::Error::other("database component is unavailable"))?;

        println!("database component resolved from the root container");

        if self.connection {
            println!("recorded query #{}", database.record_query());
        }

        let _ = self.pool;

        Ok(())
    }
}

app! {
    /// Demonstrates generated typestate lifecycle and nested typed CLI commands.
    app DaemonApplication {
        name: "example-daemon",
        protocol: overseerd::daemon::RpcPlugin,
        configs: [
            DbConfig => "app.db.reader",
            DbConfig => "app.db.writer",
            ServerConfig => "app.server",
            LoggingConfig => "logging",
        ],
        args: {
            output: OutputArgs,
        },
        commands: {
            /// Inspect validated application metadata.
            #[command(alias = "show", visible_alias = "describe", display_order = 10)]
            inspect: {
                /// Print components, dependencies, providers, and config bindings.
                registry: InspectRegistryCommand,
            },
            /// Run database administration commands.
            #[command(alias = "db", display_order = 20)]
            database: {
                /// Build the app and verify the database component.
                check: CheckDatabaseCommand,
            },
        },
        serve(_context, app, server: Cfg<ServerConfig>) {
            let server = server.snapshot();
            let transport = TcpTransport::bind((server.bind.as_str(), server.port)).await?;

            println!("{app}");
            println!(
                "daemon listening on {}:{}; press Ctrl-C to stop",
                server.bind, server.port
            );

            app.serve(transport).await?;

            Ok::<(), overseerd::daemon::Error>(())
        },
    }
}

#[tokio::main]
async fn main() -> Result<(), overseerd::CliError> {
    DaemonApplication::run().await
}
