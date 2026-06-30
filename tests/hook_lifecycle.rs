//! Proof of the builtin `Startup`/`Shutdown` lifecycle hooks: `run()` fires `Startup` before
//! waiting and `Shutdown` once a graceful stop is triggered.
#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use overseerd::config::Toml;
use overseerd::daemon::App;
use overseerd::{ConfigManager, Shutdown, Startup, component, methods};

/// Records that its startup and shutdown hooks ran.
#[component]
struct LifecycleComponent {
    #[default]
    started: AtomicUsize,
    #[default]
    stopped: AtomicUsize,
}

/// Fails startup after recording it, and records whether cleanup ran.
#[component]
struct FailingStartupComponent {
    #[default]
    started: AtomicUsize,
    #[default]
    stopped: AtomicUsize,
}

impl FailingStartupComponent {
    fn started(&self) -> usize {
        self.started.load(Ordering::SeqCst)
    }

    fn stopped(&self) -> usize {
        self.stopped.load(Ordering::SeqCst)
    }
}

impl LifecycleComponent {
    fn started(&self) -> usize {
        self.started.load(Ordering::SeqCst)
    }

    fn stopped(&self) -> usize {
        self.stopped.load(Ordering::SeqCst)
    }
}

#[methods]
impl LifecycleComponent {
    #[hook(Startup)]
    async fn on_start(&self) -> overseerd::daemon::Result<()> {
        self.started.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }

    #[hook(Shutdown)]
    async fn on_stop(&self) -> overseerd::daemon::Result<()> {
        self.stopped.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }
}

#[methods]
impl FailingStartupComponent {
    #[hook(Startup)]
    async fn on_start(&self) -> overseerd::daemon::Result<()> {
        self.started.fetch_add(1, Ordering::SeqCst);

        Err(overseerd::daemon::Error::MissingComponent(
            "startup rejected",
        ))
    }

    #[hook(Shutdown)]
    async fn on_stop(&self) -> overseerd::daemon::Result<()> {
        self.stopped.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }
}

#[tokio::test]
async fn startup_and_shutdown_hooks_fire() {
    let daemon = App::builder("lifecycle-test")
        .config_source(ConfigManager::<Toml>::empty())
        .component::<LifecycleComponent>()
        .build()
        .await
        .expect("daemon builds");

    let component = daemon
        .container()
        .get::<LifecycleComponent>()
        .expect("component built");
    let shutdown = daemon.shutdown_handle();

    assert_eq!(component.started(), 0, "not started before run");

    let task = tokio::spawn(async move {
        daemon.run().await.expect("run completes");
    });

    // Startup runs at the top of `run`, before it waits for a shutdown signal.
    let mut started = false;

    for _ in 0..50 {
        if component.started() == 1 {
            started = true;
            break;
        }

        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert!(started, "startup hook fired");
    assert_eq!(component.stopped(), 0, "not stopped while running");

    shutdown.shutdown();
    task.await.expect("run task joins");

    assert_eq!(
        component.stopped(),
        1,
        "shutdown hook fired on graceful stop"
    );
}

#[tokio::test]
async fn startup_failure_runs_shutdown_hooks_before_returning() {
    let daemon = App::builder("startup-failure-cleanup-test")
        .config_source(ConfigManager::<Toml>::empty())
        .component::<FailingStartupComponent>()
        .build()
        .await
        .expect("daemon builds");

    let component = daemon
        .container()
        .get::<FailingStartupComponent>()
        .expect("component built");

    let result = daemon.run().await;

    assert!(result.is_err(), "startup failure is returned");
    assert_eq!(component.started(), 1, "startup hook ran once");
    assert_eq!(component.stopped(), 1, "shutdown hook cleaned up");
}
