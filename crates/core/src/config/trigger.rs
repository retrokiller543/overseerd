//! Automatic config-reload triggers.
//!
//! Beyond the always-available manual [`ConfigReloader::reload`], a [`ConfigManager`] may
//! request reloads on `SIGHUP` (Unix) or on config-file changes (the `watch` feature). The
//! daemon spawns the matching background tasks at `serve`/`run` and aborts them on shutdown;
//! each just calls `reload()` and logs the outcome.
//!
//! [`ConfigManager`]: super::ConfigManager

use tokio::task::JoinHandle;
use tracing::{error, info};

use super::{ConfigReloader, ReloadTriggers};

/// Spawns the background tasks for the requested triggers, returning their handles so the
/// caller can abort them on shutdown. Unsupported requests (SIGHUP off-Unix, watching
/// without the `watch` feature) are logged and skipped.
pub(crate) fn spawn_reload_triggers(
    reloader: ConfigReloader,
    triggers: ReloadTriggers,
) -> Vec<JoinHandle<()>> {
    let mut handles = Vec::new();

    if triggers.sighup {
        #[cfg(unix)]
        handles.push(spawn_sighup(reloader.clone()));

        #[cfg(not(unix))]
        tracing::warn!(target: "overseerd::config", "reload_on_sighup is Unix-only; ignoring");
    }

    if triggers.watch {
        #[cfg(feature = "watch")]
        if let Some(handle) = spawn_watch(reloader.clone(), triggers.debounce) {
            handles.push(handle);
        }

        #[cfg(not(feature = "watch"))]
        tracing::warn!(
            target: "overseerd::config",
            "watch_config requires the `watch` feature; ignoring"
        );
    }

    handles
}

/// Runs one reload and logs its outcome.
async fn run_reload(reloader: &ConfigReloader, cause: &'static str) {
    match reloader.reload().await {
        Ok(report) => info!(
            target: "overseerd::config",
            cause,
            generation = report.generation,
            changed = report.changed.len(),
            "configuration reloaded"
        ),

        Err(error) => error!(
            target: "overseerd::config",
            cause,
            %error,
            "configuration reload failed"
        ),
    }
}

/// Reloads whenever the process receives `SIGHUP`.
#[cfg(unix)]
fn spawn_sighup(reloader: ConfigReloader) -> JoinHandle<()> {
    use tokio::signal::unix::{SignalKind, signal};

    tokio::spawn(async move {
        let mut hangup = match signal(SignalKind::hangup()) {
            Ok(hangup) => hangup,

            Err(error) => {
                error!(target: "overseerd::config", %error, "failed to install SIGHUP handler");

                return;
            }
        };

        info!(target: "overseerd::config", "reloading configuration on SIGHUP");

        while hangup.recv().await.is_some() {
            run_reload(&reloader, "sighup").await;
        }
    })
}

/// Reloads when any config source file changes, coalescing bursts over the debounce window.
/// Watches the source files' parent directories, since editors and atomic writes replace the
/// file (which would drop a file-level watch).
#[cfg(feature = "watch")]
fn spawn_watch(reloader: ConfigReloader, debounce: std::time::Duration) -> Option<JoinHandle<()>> {
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    use notify::{RecursiveMode, Watcher};

    let sources = reloader.sources();

    if sources.is_empty() {
        tracing::warn!(
            target: "overseerd::config",
            "watch_config enabled but there are no config sources to watch"
        );

        return None;
    }

    let dirs: HashSet<PathBuf> = sources
        .iter()
        .filter_map(|source| source.parent().map(Path::to_path_buf))
        .collect();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    let mut watcher =
        match notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            if result.is_ok() {
                let _ = tx.send(());
            }
        }) {
            Ok(watcher) => watcher,

            Err(error) => {
                error!(target: "overseerd::config", %error, "failed to create config file watcher");

                return None;
            }
        };

    for dir in &dirs {
        if let Err(error) = watcher.watch(dir, RecursiveMode::NonRecursive) {
            error!(
                target: "overseerd::config",
                dir = %dir.display(),
                %error,
                "failed to watch config directory"
            );
        }
    }

    let watched = dirs.len();

    Some(tokio::spawn(async move {
        // Hold the watcher for the task's lifetime; dropping it stops watching.
        let _watcher = watcher;

        info!(target: "overseerd::config", dirs = watched, "watching config files for changes");

        while rx.recv().await.is_some() {
            tokio::time::sleep(debounce).await;

            while rx.try_recv().is_ok() {}

            run_reload(&reloader, "file-change").await;
        }
    }))
}
