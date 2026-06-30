//! Framework-provided configuration property structs.
//!
//! These implement [`ConfigProperties`](overseerd_config::ConfigProperties) and derive
//! serde `Deserialize` but carry **no** `#[config(path = "..")]` auto-binding —
//! binding a missing subtree is a hard build error, so they are opt-in. A user binds
//! them explicitly, e.g. `AppBuilder::config::<ServerConfig>("server")` (or the
//! `configs:` key of the `app!{}` macro), and injects them as
//! [`Cfg<ServerConfig>`](overseerd_config::Cfg).
//!
//! They use `#[config(overseerd = ::overseerd_config)]`: this crate lives *below* the
//! facade and cannot reference `::overseerd::*`, so the macro's `overseerd =` override
//! roots the generated `ConfigProperties` impl directly at `overseerd-config`. (Without
//! a `path`, the macro emits only that impl — no descriptor or `linkme` registration —
//! so `overseerd-config`'s re-export of `ConfigProperties` is all the override needs.)

use serde::Deserialize;

use overseerd_macros::config;

/// Network binding settings for a daemon's transport, bound from a config subtree
/// and injected as [`Cfg<ServerConfig>`](overseerd_config::Cfg).
#[config(overseerd = ::overseerd_config)]
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// The host or IP address the daemon binds its listener to.
    pub bind: String,

    /// The TCP port the daemon listens on.
    pub port: u16,
}

/// Tracing/logging settings consumed by the `init_tracing` helper, bound from a
/// config subtree and injected as [`Cfg<LoggingConfig>`](overseerd_config::Cfg).
#[config(overseerd = ::overseerd_config)]
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LoggingConfig {
    /// An `EnvFilter`-style level directive (e.g. `"info"`, `"app=debug,info"`).
    pub level: String,

    /// The output format: `"full"`, `"compact"`, `"pretty"`, or `"json"`.
    pub format: String,

    /// Whether to colorize the output with ANSI escape codes.
    pub ansi: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".to_string(),
            port: 9000,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: "full".to_string(),
            ansi: true,
        }
    }
}
