use std::collections::HashMap;
use std::fs;

use crate::{MapResolver, ResolverChain};

use super::{ConfigManager, Toml};
use tempfile::Builder;

fn ambient_sentinels() -> ResolverChain {
    let values = HashMap::from([
        ("AXUM_PORT".to_string(), "49152".to_string()),
        ("OVERSEERD_PROFILES".to_string(), "ambient".to_string()),
    ]);

    ResolverChain(vec![Box::new(MapResolver(values))])
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
    let root = Builder::new()
        .prefix("overseerd-config-source-")
        .tempdir()
        .expect("create isolated config directory");

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
