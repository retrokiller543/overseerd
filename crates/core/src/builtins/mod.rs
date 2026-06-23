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

pub use config::{LoggingConfig, ServerConfig};

#[cfg(feature = "tracing-subscriber")]
pub use logging::{InitTracingError, init_tracing};

#[cfg(test)]
mod tests {
    use crate::DaemonBuilder;
    use crate::config::{ConfigManager, ConfigProperties, Toml};
    use crate::lifecycle::ShutdownHandle;

    use super::config::{LoggingConfig, ServerConfig};

    #[tokio::test]
    async fn shutdown_handle_resolves_from_root_scope() {
        let daemon = DaemonBuilder::new("builtins-test")
            .build()
            .await
            .expect("build daemon");

        let handle = daemon
            .container()
            .resolve::<ShutdownHandle>()
            .await
            .expect("ShutdownHandle resolves from the root scope");

        handle.shutdown();
    }

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
        "#;

        let tree = ConfigManager::<Toml>::from_str(TOML).expect("parse config");
        let value: LoggingConfig = tree.get("logging").expect("bind logging config");

        assert_eq!(
            value,
            LoggingConfig {
                level: "info".to_string(),
                format: "compact".to_string(),
                ansi: false,
            }
        );
    }

    #[test]
    fn config_property_names_are_stable() {
        assert_eq!(<ServerConfig as ConfigProperties>::NAME, "ServerConfig");
        assert_eq!(<LoggingConfig as ConfigProperties>::NAME, "LoggingConfig");
    }
}
