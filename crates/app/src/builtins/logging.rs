//! Process-global tracing-subscriber installation.
//!
//! Installing a subscriber is a process-wide side effect, not a DI-resolved value,
//! so this is a main-side helper rather than an injectable. It is gated behind the
//! `tracing-subscriber` feature so the dependency is only pulled in when a binary
//! actually drives logging setup.

use std::ffi::OsStr;

use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry, fmt};

use crate::builtins::config::{LoggingConfig, SpanEvents};

/// A type-erased subscriber layer that can be composed onto the framework subscriber, e.g.
/// per-run job log capture. Boxed so callers in other crates can hand in a layer the
/// (protocol-agnostic) app core has no knowledge of.
pub type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync + 'static>;

/// Errors raised while installing the global tracing subscriber from a
/// [`LoggingConfig`].
#[derive(Debug, thiserror::Error)]
pub enum InitTracingError {
    /// The configured `level` directive could not be parsed as an `EnvFilter`.
    #[error("invalid log filter from {origin} ('{filter}'): {source}")]
    Filter {
        /// The configuration source containing the invalid directive.
        origin: &'static str,
        filter: String,
        #[source]
        source: tracing_subscriber::filter::ParseError,
    },

    /// `RUST_LOG` contained bytes that are not valid Unicode.
    #[error("RUST_LOG contains non-Unicode data")]
    InvalidRustLogUnicode,

    /// The configured `format` was not one of the supported variants.
    #[error("unknown log format '{format}', expected one of: full, compact, pretty, json")]
    UnknownFormat { format: String },

    /// A global subscriber was already installed for this process.
    #[error("a global tracing subscriber is already installed")]
    AlreadyInstalled,
}

/// Installs the process-global tracing subscriber from `config`.
///
/// Builds an `EnvFilter` from the configured level directive and selects the
/// `fmt` output format. Returns [`InitTracingError::AlreadyInstalled`] if a global
/// subscriber is already in place, so callers can decide whether that is fatal.
pub fn init_tracing(config: &LoggingConfig) -> Result<(), InitTracingError> {
    init_tracing_with_layers(config, Vec::new())
}

/// Installs the process-global subscriber from `config`, composing `extra` layers on top of the
/// framework's filtered `fmt` layer.
///
/// The escape hatch for crates above the app core that need to add their own capture — e.g.
/// `overseerd-jobs` layering per-run log capture — without the app core depending on them.
/// Layers must be composed before installation, so an already-installed subscriber cannot be
/// extended after the fact; call this once, at startup.
pub fn init_tracing_with_layers(
    config: &LoggingConfig,
    extra: Vec<BoxedLayer>,
) -> Result<(), InitTracingError> {
    let rust_log = std::env::var_os(EnvFilter::DEFAULT_ENV);
    let filter = env_filter(config, rust_log.as_deref())?;

    // Compose every layer over the bare registry so each stays typed `Layer<Registry>` (boxed
    // layers cannot re-type themselves), then apply the env filter globally on top.
    let mut layers: Vec<BoxedLayer> = vec![fmt_layer(config)?];
    layers.extend(extra);

    Registry::default()
        .with(layers)
        .with(filter)
        .try_init()
        .map_err(|_| InitTracingError::AlreadyInstalled)
}

fn env_filter(
    config: &LoggingConfig,
    rust_log: Option<&OsStr>,
) -> Result<EnvFilter, InitTracingError> {
    let (filter, origin) = match rust_log {
        Some(value) => (
            value
                .to_str()
                .ok_or(InitTracingError::InvalidRustLogUnicode)?,
            EnvFilter::DEFAULT_ENV,
        ),
        None => (config.level.as_str(), "logging.level"),
    };

    EnvFilter::try_new(filter).map_err(|source| InitTracingError::Filter {
        origin,
        filter: filter.to_owned(),
        source,
    })
}

/// Builds the boxed `fmt` layer for the configured output format, honouring the ansi setting.
fn fmt_layer(config: &LoggingConfig) -> Result<BoxedLayer, InitTracingError> {
    let base = fmt::layer()
        .with_ansi(config.ansi)
        .with_span_events(fmt_span(config.span_events))
        .with_target(config.target)
        .with_level(config.level_display)
        .with_thread_ids(config.thread_ids)
        .with_thread_names(config.thread_names)
        .with_file(config.file)
        .with_line_number(config.line_number);

    let layer = match config.format.as_str() {
        "full" => base.boxed(),
        "compact" => base.compact().boxed(),
        "pretty" => base.pretty().boxed(),
        "json" => base
            .json()
            .flatten_event(config.flatten_event)
            .with_current_span(config.current_span)
            .with_span_list(config.current_span)
            .boxed(),

        other => {
            return Err(InitTracingError::UnknownFormat {
                format: other.to_string(),
            });
        }
    };

    Ok(layer)
}

fn fmt_span(events: SpanEvents) -> FmtSpan {
    match events {
        SpanEvents::None => FmtSpan::NONE,
        SpanEvents::New => FmtSpan::NEW,
        SpanEvents::Enter => FmtSpan::ENTER,
        SpanEvents::Exit => FmtSpan::EXIT,
        SpanEvents::Close => FmtSpan::CLOSE,
        SpanEvents::Active => FmtSpan::ACTIVE,
        SpanEvents::Full => FmtSpan::FULL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::config::LoggingConfig;

    #[test]
    fn unknown_format_is_rejected() {
        let config = LoggingConfig {
            level: "info".to_string(),
            format: "xml".to_string(),
            ansi: false,
            ..LoggingConfig::default()
        };

        let result = init_tracing(&config);

        assert!(matches!(
            result,
            Err(InitTracingError::UnknownFormat { .. })
        ));
    }

    #[test]
    fn rust_log_overrides_configured_level_and_targets() {
        let config = LoggingConfig {
            level: "invalid[".to_owned(),
            ..LoggingConfig::default()
        };

        let result = env_filter(&config, Some(OsStr::new("warn,overseerd=trace")));

        assert!(result.is_ok());
    }

    #[test]
    fn configured_level_is_used_without_rust_log() {
        let config = LoggingConfig {
            level: "invalid[".to_owned(),
            ..LoggingConfig::default()
        };

        let result = env_filter(&config, None);

        assert!(matches!(
            result,
            Err(InitTracingError::Filter {
                origin: "logging.level",
                ..
            })
        ));
    }
}
