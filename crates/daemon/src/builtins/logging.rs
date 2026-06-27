//! Process-global tracing-subscriber installation.
//!
//! Installing a subscriber is a process-wide side effect, not a DI-resolved value,
//! so this is a main-side helper rather than an injectable. It is gated behind the
//! `tracing-subscriber` feature so the dependency is only pulled in when a binary
//! actually drives logging setup.

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

use crate::builtins::config::LoggingConfig;

/// Errors raised while installing the global tracing subscriber from a
/// [`LoggingConfig`].
#[derive(Debug, thiserror::Error)]
pub enum InitTracingError {
    /// The configured `level` directive could not be parsed as an `EnvFilter`.
    #[error("invalid log level filter '{filter}': {source}")]
    Filter {
        filter: String,
        #[source]
        source: tracing_subscriber::filter::ParseError,
    },

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
    let filter = EnvFilter::try_new(&config.level).map_err(|source| InitTracingError::Filter {
        filter: config.level.clone(),
        source,
    })?;

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(config.ansi)
        .with_span_events(FmtSpan::NONE);

    let installed = match config.format.as_str() {
        "full" => builder.try_init(),
        "compact" => builder.compact().try_init(),
        "pretty" => builder.pretty().try_init(),
        "json" => builder.json().try_init(),

        other => {
            return Err(InitTracingError::UnknownFormat {
                format: other.to_string(),
            });
        }
    };

    installed.map_err(|_| InitTracingError::AlreadyInstalled)
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
        };

        let result = init_tracing(&config);

        assert!(matches!(
            result,
            Err(InitTracingError::UnknownFormat { .. })
        ));
    }
}
