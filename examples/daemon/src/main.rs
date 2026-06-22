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

use overseer::Daemon;

use crate::components::Config;

#[tokio::main]
async fn main() -> overseer::Result<()> {
    // `auto_discover` collects every `#[component]`/`#[service]`/`#[handlers]`
    // in the binary; `Config` is the one instance constructed by hand.
    let daemon = Daemon::builder("example-daemon")
        .auto_discover()
        .with_component(Config {
            greeting: "Hello".to_string(),
        })
        .build()
        .await?;

    println!("{daemon}");

    Ok(())
}
