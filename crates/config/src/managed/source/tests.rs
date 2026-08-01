use std::collections::HashMap;
use std::fs;

use super::{ConfigManager, Toml};
use crate::managed::ConfigError;
use crate::{MapResolver, ResolverChain};
use tempfile::{Builder, TempDir};

fn temp_config_dir() -> TempDir {
    Builder::new()
        .prefix("overseerd-config-source-")
        .tempdir()
        .expect("create isolated config directory")
}

fn ambient_sentinels() -> ResolverChain {
    let values = HashMap::from([
        ("AXUM_PORT".to_string(), "49152".to_string()),
        ("OVERSEERD_PROFILES".to_string(), "ambient".to_string()),
    ]);

    ResolverChain(vec![Box::new(MapResolver(values))])
}

#[test]
fn exact_file_profiles_override_in_order_and_retain_sources() {
    let directory = temp_config_dir();
    let base = directory.path().join("custom.toml");
    let first = directory.path().join("custom-first.toml");
    let second = directory.path().join("custom-second.toml");

    fs::write(&base, "[app]\nvalue = 1\nbase = true\n").expect("write base config");
    fs::write(&first, "[app]\nvalue = 2\nfirst = true\n").expect("write first profile");
    fs::write(&second, "[app]\nvalue = 3\n").expect("write second profile");

    let manager = ConfigManager::<Toml>::load_file(
        &base,
        &[
            String::from("first"),
            String::from("missing"),
            String::from("second"),
        ],
    )
    .expect("load exact config and profiles");

    assert_eq!(manager.get::<i64>("app.value").expect("read value"), 3);
    assert!(manager.get::<bool>("app.base").expect("read base value"));
    assert!(
        manager
            .get::<bool>("app.first")
            .expect("read profile value")
    );
    assert_eq!(manager.sources(), [base, first, second]);
}

#[test]
fn exact_file_requires_a_supported_extension() {
    let directory = temp_config_dir();
    let path = directory.path().join("application.json");

    fs::write(&path, "{}").expect("write unsupported config");

    let error = match ConfigManager::<Toml>::load_file(&path, &[]) {
        Ok(_) => panic!("unsupported config extension was accepted"),
        Err(error) => error,
    };

    assert!(
        matches!(error, ConfigError::UnsupportedFormat { path: error_path } if error_path == path)
    );
}

#[test]
fn explicit_directory_profiles_ignore_environment_resolution() {
    let directory = temp_config_dir();

    fs::write(directory.path().join("application.toml"), "value = 1\n").expect("write base config");
    fs::write(directory.path().join("application-cli.toml"), "value = 2\n")
        .expect("write CLI profile");

    let manager = ConfigManager::<Toml>::load_in_explicit(directory.path(), &[String::from("cli")])
        .expect("load explicit profiles");

    assert_eq!(manager.get::<i64>("value").expect("read value"), 2);
    assert_eq!(manager.sources().len(), 2);
}

#[test]
fn empty_explicit_chain_ignores_an_in_memory_axum_sentinel() {
    const CONFIG: &str = "[axum]\nport = \"${AXUM_PORT:3000}\"\n";

    let ambient = ConfigManager::<Toml>::from_str(CONFIG)
        .expect("parse ambient control")
        .with_resolvers(ambient_sentinels());
    let isolated = ConfigManager::<Toml>::from_str(CONFIG)
        .expect("parse isolated config")
        .with_resolvers(ambient_sentinels())
        .with_resolvers(ResolverChain::empty());

    assert_eq!(
        ambient.get::<u16>("axum.port").expect("ambient port"),
        49152
    );
    assert_eq!(
        isolated.get::<u16>("axum.port").expect("default port"),
        3000
    );
}

#[test]
fn empty_explicit_chain_ignores_an_in_memory_profile_sentinel() {
    let root = temp_config_dir();

    fs::write(root.path().join("application.toml"), "value = \"base\"\n")
        .expect("write base config");
    fs::write(
        root.path().join("application-ambient.toml"),
        "value = \"ambient\"\n",
    )
    .expect("write ambient profile");

    let ambient =
        ConfigManager::<Toml>::load_in_with_resolvers(root.path(), &[], ambient_sentinels())
            .expect("load ambient control");
    let isolated =
        ConfigManager::<Toml>::load_in_with_resolvers(root.path(), &[], ResolverChain::empty())
            .expect("load isolated config");

    assert_eq!(
        ambient.get::<String>("value").expect("ambient value"),
        "ambient"
    );
    assert_eq!(isolated.get::<String>("value").expect("base value"), "base");
    assert_eq!(isolated.sources(), &[root.path().join("application.toml")]);
}
