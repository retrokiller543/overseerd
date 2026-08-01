//! End-to-end proof of manual config reloading: a file-backed app injects two
//! `Cfg<T>` bindings, one source value changes, and a reload re-publishes **only**
//! the changed binding — the unchanged one keeps its exact `Arc` (no spurious swap),
//! and a snapshot taken before the reload stays pinned to the old value.

use std::fs;
use std::sync::Arc;

use overseerd::config::Toml;
use overseerd::dirs::{Config, DirectoriesManager};
use overseerd::{App, Cfg, ConfigManager, component, config};
use overseerd_config::ResolverChain;
use serde::Deserialize;
use tempfile::TempDir;

#[config(path = "svc")]
#[derive(Deserialize)]
struct SvcCfg {
    value: u32,
}

#[config(path = "other")]
#[derive(Deserialize)]
struct OtherCfg {
    value: u32,
}

/// Holds two config bindings so a reload can be observed to touch only one.
#[component]
struct Consumer {
    #[config("svc")]
    svc: Cfg<SvcCfg>,
    #[config("other")]
    other: Cfg<OtherCfg>,
}

impl Consumer {
    fn svc(&self) -> &Cfg<SvcCfg> {
        &self.svc
    }

    fn other(&self) -> &Cfg<OtherCfg> {
        &self.other
    }
}

fn temp_config_dir() -> TempDir {
    tempfile::Builder::new()
        .prefix("overseerd-config-reload-")
        .tempdir()
        .expect("create temp config dir")
}

#[tokio::test]
async fn reload_swaps_only_the_changed_binding() {
    let root = temp_config_dir();
    let dirs = DirectoriesManager::from_path(root.path().to_path_buf());
    let config_dir = dirs.dir::<Config>();
    let config_file = config_dir.path().join("application.toml");

    fs::create_dir_all(config_dir.path()).expect("create config subdir");
    fs::write(&config_file, "[svc]\nvalue = 1\n\n[other]\nvalue = 100\n").expect("write config");

    let manager =
        ConfigManager::<Toml>::load_in_with_resolvers(&config_dir, &[], ResolverChain::empty())
            .expect("load config");

    let daemon = App::<()>::builder("config-reload-test")
        .config_source(manager)
        .auto_discover()
        .build()
        .await
        .expect("daemon builds");

    let consumer = daemon
        .container()
        .get::<Consumer>()
        .expect("Consumer constructed");

    let svc_before = consumer.svc().snapshot();
    let other_before = consumer.other().snapshot();

    assert_eq!(svc_before.value, 1, "svc starts at the file value");
    assert_eq!(other_before.value, 100, "other starts at the file value");

    fs::write(&config_file, "[svc]\nvalue = 2\n\n[other]\nvalue = 100\n").expect("rewrite config");

    let report = daemon
        .config_reloader()
        .reload()
        .await
        .expect("reload succeeds");

    assert_eq!(
        report.generation, 1,
        "first successful reload is generation 1"
    );
    assert_eq!(report.changed.len(), 1, "only one binding changed");
    assert_eq!(report.changed[0].path, "svc", "the changed binding is svc");

    assert_eq!(
        consumer.svc().get().value,
        2,
        "the changed binding observes the new value"
    );
    assert_eq!(
        consumer.other().get().value,
        100,
        "the unchanged binding keeps its value"
    );

    assert!(
        Arc::ptr_eq(&other_before, &consumer.other().snapshot()),
        "the unchanged binding was not re-published (same Arc, no spurious swap)"
    );
    assert!(
        !Arc::ptr_eq(&svc_before, &consumer.svc().snapshot()),
        "the changed binding was actually swapped"
    );
    assert_eq!(
        svc_before.value, 1,
        "a snapshot taken before the reload stays pinned to the old value"
    );

    let unchanged = daemon
        .config_reloader()
        .reload()
        .await
        .expect("second reload succeeds");

    assert_eq!(unchanged.generation, 2, "generation advances every reload");
    assert!(
        unchanged.changed.is_empty(),
        "re-reading identical sources changes nothing"
    );
}
