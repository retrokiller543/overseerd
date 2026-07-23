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
#[non_exhaustive]
pub struct LoggingConfig {
    /// An `EnvFilter`-style level directive (e.g. `"info"`, `"app=debug,info"`).
    pub level: String,

    /// The formatter used for tracing output.
    pub format: LogFormat,

    /// Whether to colorize the output with ANSI escape codes.
    pub ansi: bool,

    /// Span lifecycle events emitted by the formatter.
    #[serde(default)]
    pub span_events: SpanEvents,

    /// Whether event targets are included in formatted output.
    #[serde(default = "default_true")]
    pub target: bool,

    /// Whether event levels are included in formatted output.
    #[serde(default = "default_true")]
    pub level_display: bool,

    /// Whether thread IDs are included in formatted output.
    #[serde(default)]
    pub thread_ids: bool,

    /// Whether thread names are included in formatted output.
    #[serde(default)]
    pub thread_names: bool,

    /// Whether source file paths are included in formatted output.
    #[serde(default)]
    pub file: bool,

    /// Whether source line numbers are included in formatted output.
    #[serde(default)]
    pub line_number: bool,

    /// Whether JSON output flattens event fields into the root object.
    #[serde(default)]
    pub flatten_event: bool,

    /// Whether JSON output includes the current span and span list.
    #[serde(default = "default_true")]
    pub current_span: bool,
}

impl LoggingConfig {
    /// Creates logging settings with the requested filter and default formatter options.
    pub fn new(level: impl Into<String>) -> Self {
        Self {
            level: level.into(),
            ..Self::default()
        }
    }

    /// Selects the formatter output style.
    pub fn with_format(mut self, format: LogFormat) -> Self {
        self.format = format;

        self
    }

    /// Controls ANSI color output.
    pub fn with_ansi(mut self, ansi: bool) -> Self {
        self.ansi = ansi;

        self
    }

    /// Selects synthetic span lifecycle events.
    pub fn with_span_events(mut self, span_events: SpanEvents) -> Self {
        self.span_events = span_events;

        self
    }
}

/// Formatter used for tracing output.
#[derive(Deserialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LogFormat {
    /// Standard human-readable event output.
    #[default]
    Full,
    /// Condensed human-readable event output.
    Compact,
    /// Multi-line human-readable event output.
    Pretty,
    /// Structured JSON event output.
    Json,
}

/// Span lifecycle events emitted by tracing formatters.
#[derive(Deserialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SpanEvents {
    /// Do not synthesize span lifecycle events.
    #[default]
    None,
    /// Emit an event when each span is created.
    New,
    /// Emit an event whenever a span is entered.
    Enter,
    /// Emit an event whenever a span is exited.
    Exit,
    /// Emit an event when each span closes.
    Close,
    /// Emit enter and exit events.
    Active,
    /// Emit new, enter, exit, and close events.
    Full,
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
            format: LogFormat::Full,
            ansi: true,
            span_events: SpanEvents::None,
            target: true,
            level_display: true,
            thread_ids: false,
            thread_names: false,
            file: false,
            line_number: false,
            flatten_event: false,
            current_span: true,
        }
    }
}

const fn default_true() -> bool {
    true
}
