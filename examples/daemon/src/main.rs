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
use overseerd::builtins::init_tracing;
use overseerd::config::Toml;
use overseerd::{ConfigManager, DirectoriesManager, LoggingConfig, ServerConfig, daemon};

#[tokio::main]
async fn main() -> overseerd::Result<()> {
    const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

    let dir_manager = DirectoriesManager::from_path(CRATE_PATH.into());

    // Build the merged config first. Its `${VAR:default}` placeholders resolve
    // against the environment as each subtree is deserialized; `with_directories`
    // also wires the `${@kind}` directory namespace so config can reference, e.g.,
    // the runtime directory.
    let config =
        ConfigManager::<Toml>::load_in(&dir_manager.dir(), &[])?.with_directories(&dir_manager);

    init_tracing(&config.get("logging")?).ok();

    // Configure the transport from config before the daemon is assembled. `get_config`
    // applies the type's `#[default]` fields, so `socket` (omitted from the file) falls
    // back to its templated default under the runtime directory.
    let server: AppServer = config.get_config::<AppServer>("app.server")?;
    println!("server would bind to {}", server.addr);
    println!("server socket resolves to {}", server.socket.display());

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
            LoggingConfig => "logging"
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
