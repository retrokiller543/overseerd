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

mod commands;
mod components;
mod notifiers;
mod service;

use crate::commands::{CheckDatabaseCommand, InspectRegistryCommand, OutputArgs};
use crate::components::DbConfig;
use overseerd::{Cfg, LoggingConfig, ServerConfig, TcpTransport, app};

app! {
    /// Demonstrates generated typestate lifecycle and nested typed CLI commands.
    app DaemonApplication {
        name: "example-daemon",
        protocol: overseerd::daemon::Rpc,
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
