//! Proof that a rejecting `#[hook(ConfigReload)]` aborts the whole reload (two-phase
//! all-or-nothing): nothing is committed and the live config keeps its old value.
#![cfg(feature = "daemon")]
#![allow(dead_code)]

use std::fs;

use overseerd::config::Toml;
use overseerd::daemon::App;
use overseerd::dirs::{Config, DirectoriesManager};
use overseerd::{
    Cfg, CfgNext, ConfigManager, ConfigReload, ConfigReloadError, HookOutcome, component, config,
    methods,
};
use overseerd_config::ResolverChain;
use serde::Deserialize;
use tempfile::TempDir;

#[config(path = "svc")]
#[derive(Deserialize)]
struct SvcCfg {
    value: u32,
}

/// Always rejects a reload of `svc`.
#[component]
struct Rejector {
    #[config("svc")]
    svc: Cfg<SvcCfg>,
}

impl Rejector {
    fn committed(&self) -> u32 {
        self.svc.get().value
    }
}

#[methods]
impl Rejector {
    #[hook(ConfigReload)]
    async fn on_reload(
        &self,
        #[config("svc")] _next: CfgNext<SvcCfg>,
    ) -> overseerd::daemon::Result<HookOutcome> {
        Err(overseerd::daemon::Error::MissingComponent(
            "rejected by test hook",
        ))
    }
}

fn temp_config_dir() -> TempDir {
    tempfile::Builder::new()
        .prefix("overseerd-hook-abort-")
        .tempdir()
        .expect("create temp config dir")
}

#[tokio::test]
async fn a_rejecting_hook_aborts_the_reload() {
    let root = temp_config_dir();
    let dirs = DirectoriesManager::from_path(root.path().to_path_buf());
    let config_dir = dirs.dir::<Config>();
    let config_file = config_dir.path().join("application.toml");

    fs::create_dir_all(config_dir.path()).expect("create config subdir");
    fs::write(&config_file, "[svc]\nvalue = 1\n").expect("write config");

    let manager =
        ConfigManager::<Toml>::load_in_with_resolvers(&config_dir, &[], ResolverChain::empty())
            .expect("load config");

    let daemon = App::builder("hook-abort-test")
        .config_source(manager)
        .auto_discover()
        .build()
        .await
        .expect("daemon builds");

    let rejector = daemon
        .container()
        .get::<Rejector>()
        .expect("Rejector built");

    assert_eq!(rejector.committed(), 1, "starts at the file value");

    fs::write(&config_file, "[svc]\nvalue = 2\n").expect("rewrite config");

    let result = daemon.config_reloader().reload().await;

    assert!(
        matches!(result, Err(ConfigReloadError::Hook { .. })),
        "the rejecting hook surfaces as a hook error: {result:?}"
    );
    assert_eq!(
        rejector.committed(),
        1,
        "the value was NOT committed — the rejected reload rolled back"
    );

    // The reloader is still usable; the generation did not advance on the aborted reload.
    assert_eq!(
        daemon.config_reloader().generation(),
        0,
        "no successful reload yet"
    );
}
