//! A small but complete Overseer daemon, demonstrating the dependency-injection
//! surface across modules — and validated at build time by `build.rs`.
//!
//! Run it to assemble the daemon and print the discovered registry (components,
//! their dependencies, services, and RPCs):
//!
//! ```text
//! cargo run -p overseer-example-daemon
//! ```

mod components;
mod notifiers;
mod service;

use overseer::daemon;

use crate::components::Config;
use crate::service::Notifications;

#[tokio::main]
async fn main() -> overseer::Result<()> {
    let config = Config {
        greeting: "Hello, world!".to_string(),
    };

    // `daemon!` assembles the daemon — auto-discovers every
    // `#[component]`/`#[service]`/`#[handlers]` and registers `Config` (the one
    // instance built by hand) — and, under `di-check`, asserts at compile time
    // that the listed services' dependency graphs are fully satisfied.
    let daemon = daemon! {
        name: "example-daemon",
        services: [Notifications],
        components: [config],
    }
    .build()
    .await?;

    println!("{daemon}");

    Ok(())
}
