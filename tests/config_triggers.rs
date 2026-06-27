//! Phase 4 triggers: `ConfigManager` carries the opt-in reload triggers (config lives on the
//! manager, never the daemon), the `app!` macro can construct + configure a manager from a
//! per-manager config block, and — under the `watch` feature — a file change drives a reload.
#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use overseerd::config::Toml;
use overseerd::dirs::{Config, DirectoriesManager};
use overseerd::{ConfigManager, app};

#[cfg(feature = "watch")]
use overseerd::App;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("overseerd-triggers-{tag}-{}", std::process::id()));

    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");

    dir
}

#[test]
fn config_manager_carries_its_triggers() {
    let manager = ConfigManager::<Toml>::empty()
        .reload_on_sighup()
        .watch_config()
        .config_reload_debounce(Duration::from_millis(123));

    let triggers = manager.triggers();

    assert!(triggers.sighup, "sighup requested");
    assert!(triggers.watch, "watch requested");
    assert_eq!(triggers.debounce, Duration::from_millis(123));
}

#[tokio::test]
async fn daemon_macro_builds_a_configured_manager_from_a_block() -> overseerd::Result<()> {
    let root = temp_dir("macro");
    let dirs = DirectoriesManager::from_path(root);

    fs::create_dir_all(dirs.dir::<Config>().path()).expect("create config dir");
    fs::write(dirs.dir::<Config>().join("application.toml"), "").expect("write config");

    // `config` is a block (no instance): the macro loads it from the `directories` instance
    // and applies the triggers to the manager.
    let built = app! {
        name: "trigger-macro-test",
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
async fn watching_a_source_file_triggers_a_reload() {
    let root = temp_dir("watch");
    let dirs = DirectoriesManager::from_path(root);
    let config_dir = dirs.dir::<Config>();
    let config_file = config_dir.path().join("application.toml");

    fs::create_dir_all(config_dir.path()).expect("create config dir");
    fs::write(&config_file, "[demo]\nvalue = 1\n").expect("write config");

    let manager = ConfigManager::<Toml>::load_from(&dirs, &[])
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

    // `run` spawns the watch trigger task; drive it in the background.
    let task = tokio::spawn(async move {
        let _ = daemon.run().await;
    });

    // Let the watcher install before editing.
    tokio::time::sleep(Duration::from_millis(200)).await;
    fs::write(&config_file, "[demo]\nvalue = 2\n").expect("rewrite config");

    let mut reloaded = false;

    for _ in 0..60 {
        if reloader.generation() > before {
            reloaded = true;
            break;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    shutdown.shutdown();
    let _ = task.await;

    assert!(reloaded, "a config file change triggered a reload");
}
