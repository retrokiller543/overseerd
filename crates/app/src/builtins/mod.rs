//! Framework-provided builtin injectables, config structs, and helpers.
//!
//! v1 is minimal infra-only:
//! - the seeded [`ShutdownHandle`](crate::lifecycle::ShutdownHandle) singleton
//!   injectable (impls live in [`shutdown`]),
//! - opt-in config property structs [`ServerConfig`] / [`LoggingConfig`],
//! - the feature-gated [`init_tracing`] subscriber helper.

pub mod config;
pub mod shutdown;

#[cfg(feature = "tracing-subscriber")]
pub mod logging;

pub use config::{LogFormat, LoggingConfig, ServerConfig, SpanEvents};

#[cfg(feature = "tracing-subscriber")]
pub use logging::{BoxedLayer, InitTracingError, init_tracing, init_tracing_with_layers};

#[cfg(test)]
mod tests {
    use overseerd_config::{ConfigManager, ConfigProperties, Toml};

    use super::config::{LogFormat, LoggingConfig, ServerConfig, SpanEvents};

    #[test]
    fn server_config_round_trips_a_subtree() {
        const TOML: &str = r#"
            [server]
            bind = "${BUILTINS_TEST_BIND:0.0.0.0}"
            port = 8080
        "#;

        let tree = ConfigManager::<Toml>::from_str(TOML).expect("parse config");
        let value: ServerConfig = tree.get("server").expect("bind server config");

        assert_eq!(
            value,
            ServerConfig {
                bind: "0.0.0.0".to_string(),
                port: 8080,
            }
        );
    }

    #[test]
    fn logging_config_round_trips_a_subtree() {
        const TOML: &str = r#"
            [logging]
            level = "${BUILTINS_TEST_LEVEL:info}"
            format = "compact"
            ansi = false

            span_events = "active"
            target = false
            level_display = false
            thread_ids = true
            thread_names = true
            file = true
            line_number = true
            flatten_event = true
            current_span = false
        "#;

        let tree = ConfigManager::<Toml>::from_str(TOML).expect("parse config");
        let value: LoggingConfig = tree.get("logging").expect("bind logging config");

        assert_eq!(
            value,
            LoggingConfig {
                level: "info".to_string(),
                format: LogFormat::Compact,
                ansi: false,
                span_events: SpanEvents::Active,
                target: false,
                level_display: false,
                thread_ids: true,
                thread_names: true,
                file: true,
                line_number: true,
                flatten_event: true,
                current_span: false,
            }
        );
    }

    #[test]
    fn unknown_log_format_is_rejected_during_extraction() {
        let tree = ConfigManager::<Toml>::from_str(
            r#"
                [logging]
                level = "info"
                format = "xml"
                ansi = false
            "#,
        )
        .expect("parse config");
        let result = tree.get::<LoggingConfig>("logging");

        assert!(result.is_err());
    }

    #[test]
    fn config_property_names_are_stable() {
        assert_eq!(<ServerConfig as ConfigProperties>::NAME, "ServerConfig");
        assert_eq!(<LoggingConfig as ConfigProperties>::NAME, "LoggingConfig");
    }
}
