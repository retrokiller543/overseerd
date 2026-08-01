//! Phase 4 triggers: `ConfigManager` carries the opt-in reload triggers (config lives on the
//! manager, never the daemon), the `app!` macro can construct + configure a manager from a
//! per-manager config block, and — under the `watch` feature — a file change drives a reload.
#![allow(dead_code)]

use std::fs;
use std::time::Duration;

use overseerd::ConfigManager;
use overseerd::app;
use overseerd::config::Toml;
use overseerd::dirs::{Config, DirectoriesManager};
use overseerd_config::ResolverChain;
use overseerd_test_utils::AbortOnDropTask;
use tempfile::TempDir;

#[cfg(feature = "watch")]
use overseerd::daemon::App;

fn temp_dir(tag: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(&format!("overseerd-triggers-{tag}-"))
        .tempdir()
        .expect("create temp dir")
}

#[test]
fn config_manager_carries_its_triggers() {
    let manager = ConfigManager::<Toml>::empty()
        .with_resolvers(ResolverChain::empty())
        .reload_on_sighup()
        .watch_config()
        .config_reload_debounce(Duration::from_millis(123));

    let triggers = manager.triggers();

    assert!(triggers.sighup, "sighup requested");
    assert!(triggers.watch, "watch requested");
    assert_eq!(triggers.debounce, Duration::from_millis(123));
}

#[tokio::test]
async fn daemon_macro_builds_a_configured_manager_from_a_block() -> overseerd::daemon::Result<()> {
    let root = temp_dir("macro");
    let dirs = DirectoriesManager::from_path(root.path().to_path_buf());

    fs::create_dir_all(dirs.dir::<Config>().path()).expect("create config dir");
    fs::write(dirs.dir::<Config>().join("application.toml"), "").expect("write config");

    // `config` is a block (no instance): the macro loads it from the `directories` instance
    // and applies the triggers to the manager.
    let built = app! {
        name: "trigger-macro-test",
        protocol: overseerd::daemon::RpcPlugin,
        managers: {
            directories: dirs,
            config: { sighup: true, debounce: Duration::from_millis(50) },
        },
    }
    .build()
    .await?;

    // The reloader is always present; a manual reload still works.
    let report = built
        .config_reloader()
        .reload()
        .await
        .expect("manual reload works");

    assert!(report.changed.is_empty(), "nothing changed on first reload");

    Ok(())
}

#[cfg(feature = "watch")]
#[tokio::test]
async fn watching_a_source_file_triggers_a_reload() -> Result<(), Box<dyn std::error::Error>> {
    let root = temp_dir("watch");
    let dirs = DirectoriesManager::from_path(root.path().to_path_buf());
    let config_dir = dirs.dir::<Config>();
    let config_file = config_dir.path().join("application.toml");

    fs::create_dir_all(config_dir.path()).expect("create config dir");
    fs::write(&config_file, "[demo]\nvalue = 1\n").expect("write config");

    let manager =
        ConfigManager::<Toml>::load_from_with_resolvers(&dirs, &[], ResolverChain::empty())
            .expect("load config")
            .watch_config()
            .config_reload_debounce(Duration::from_millis(50));

    let daemon = App::builder("watch-test")
        .config_source(manager)
        .build()
        .await
        .expect("daemon builds");

    let reloader = daemon.config_reloader();
    let shutdown = daemon.shutdown_handle();
    let before = reloader.generation();

    let mut task = AbortOnDropTask::spawn("watch daemon", daemon.run());
    let mut daemon_exit = None;
    let mut reloaded = false;

    for value in 2..=31 {
        fs::write(&config_file, format!("[demo]\nvalue = {value}\n")).expect("rewrite config");

        tokio::select! {
            result = task.join() => {
                daemon_exit = Some(result);
                break;
            }
            () = tokio::time::sleep(Duration::from_millis(100)) => {}
        }

        if reloader.generation() > before {
            reloaded = true;
            break;
        }
    }

    let exited_early = daemon_exit.is_some() || task.is_finished();

    shutdown.shutdown();
    let daemon_result = match daemon_exit {
        Some(result) => result,
        None => task.join_with_timeout(Duration::from_secs(2)).await,
    };

    daemon_result?;

    assert!(!exited_early, "daemon exited before a reload was observed");
    assert!(reloaded, "a config file change triggered a reload");

    Ok(())
}
