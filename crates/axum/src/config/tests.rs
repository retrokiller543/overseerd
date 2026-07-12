use std::net::{IpAddr, SocketAddr};

use overseerd_config::{ConfigManager, Toml};

use super::{AXUM_CONFIG_PATH, AxumConfig};

#[test]
fn defaults_materialize_without_an_axum_subtree() {
    let manager = ConfigManager::<Toml>::empty().with_config::<AxumConfig>(AXUM_CONFIG_PATH);
    let config = manager
        .get_config::<AxumConfig>(AXUM_CONFIG_PATH)
        .expect("default axum config");

    assert_eq!(config, AxumConfig::default());
    assert_eq!(
        config.socket_addr(),
        SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 3000)
    );
}

#[test]
fn configured_listener_overrides_the_defaults() {
    let manager = ConfigManager::<Toml>::from_str(
        r#"
            [axum]
            bind = "0.0.0.0"
            port = 8080
            max_request_body_bytes = 1048576
            request_timeout_ms = 15000
            graceful_shutdown_timeout_ms = 5000
        "#,
    )
    .expect("parse config")
    .with_config::<AxumConfig>(AXUM_CONFIG_PATH);

    let config = manager
        .get_config::<AxumConfig>(AXUM_CONFIG_PATH)
        .expect("configured axum listener");

    assert_eq!(
        config.socket_addr(),
        SocketAddr::new(IpAddr::from([0, 0, 0, 0]), 8080)
    );
    assert_eq!(config.max_request_body_bytes, 1_048_576);
    assert_eq!(config.request_timeout_ms, 15_000);
    assert_eq!(config.graceful_shutdown_timeout_ms, 5_000);
}
