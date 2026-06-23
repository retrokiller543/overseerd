//! A small but complete Overseerd daemon, demonstrating the dependency-injection
//! surface across modules — including config bound from a merged tree — and
//! validated at build time by `build.rs`.
//!
//! Run it to assemble the daemon and print the discovered registry (components,
//! their dependencies, services, and RPCs):
//!
//! ```text
//! cargo run -p overseerd-example-daemon
//! ```

mod components;
mod notifiers;
mod service;

use crate::components::{AppServer, DbConfig};
use crate::service::Notifications;
use overseerd::config::Toml;
use overseerd::{ConfigManager, DirectoriesManager, ServerConfig, daemon};

#[tokio::main]
async fn main() -> overseerd::Result<()> {
    const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

    let dir_manager = DirectoriesManager::from_path(CRATE_PATH.into());

    // Build the merged config first. Its `${VAR:default}` placeholders resolve
    // against the environment as each subtree is deserialized.
    let config = ConfigManager::<Toml>::load_in(&dir_manager.dir(), &[])?;

    // Configure the transport from config before the daemon is assembled.
    let server: AppServer = config.get("app.server")?;
    println!("server would bind to {}", server.addr);

    // `app.greet` auto-registers via its `#[config(path = "app.greet")]`; the two
    // `DbConfig` bindings share one type at different paths, so they are listed
    // explicitly. The framework `ServerConfig` builtin carries no auto-binding, so
    // it is bound here at `app.server`. The supplied config source backs them all.
    let daemon = daemon! {
        name: "example-daemon",
        services: [Notifications],
        configs: [
            DbConfig => "app.db.reader",
            DbConfig => "app.db.writer",
            ServerConfig => "app.server",
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
