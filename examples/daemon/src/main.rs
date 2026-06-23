//! A small but complete Overseerd daemon, demonstrating:
//!
//! 1. A `clap` CLI with `ServiceCommand` embedded — install, start, stop, etc.
//! 2. A component that also provides `dyn HealthCheck` (auto-registered via DI).
//! 3. The `run` subcommand path (direct foreground) vs. the install/start path.
//!
//! Run it to assemble the daemon and print the discovered registry:
//!
//! ```text
//! cargo run -p overseerd-example-daemon
//! cargo run -p overseerd-example-daemon -- install
//! cargo run -p overseerd-example-daemon -- status
//! cargo run -p overseerd-example-daemon -- run
//! ```

mod components;
mod notifiers;
mod service;

use crate::components::{AppServer, DbConfig};
use crate::service::Notifications;
use clap::Parser;
use overseerd::config::Toml;
use overseerd::{ConfigManager, DirectoriesManager, daemon};
use overseerd_service_manager::{ServiceCommand, ServiceManager};

#[derive(Parser)]
#[command(name = "example-daemon", about = "Overseerd example daemon")]
struct Cli {
    #[command(subcommand)]
    command: Option<ServiceCommand>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Handle service management subcommands without starting the daemon.
    if let Some(cmd) = cli.command {
        let manager = ServiceManager::new("example-daemon")
            .description("Overseerd example daemon")
            .build()?;

        let run_inline = !cmd.execute(&manager)?;

        if !run_inline {
            return Ok(());
        }
    }

    // Either `run` was specified or no subcommand was given — start in-process.
    const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

    let dir_manager = DirectoriesManager::from_path(CRATE_PATH.into());

    let config = ConfigManager::<Toml>::load_in(&dir_manager.dir(), &[])?;

    let server: AppServer = config.get("app.server")?;
    println!("server would bind to {}", server.addr);

    let daemon = daemon! {
        name: "example-daemon",
        services: [Notifications],
        configs: [
            DbConfig => "app.db.reader",
            DbConfig => "app.db.writer",
        ],
        managers: {
            config: config,
            directories: dir_manager,
        }
    }
    .build()
    .await?;

    println!("{daemon}");

    Ok(())
}

