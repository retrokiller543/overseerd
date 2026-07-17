//! Proof of the builtin `Startup`/`Shutdown` lifecycle hooks: `run()` fires `Startup` before
//! waiting and `Shutdown` once a graceful stop is triggered.
#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::FutureExt;
use overseerd::config::Toml;
use overseerd::daemon::App;
use overseerd::{ConfigManager, Shutdown, Startup, component, methods};
use overseerd_app::{
    AppRegistry, AppRuntime, Plugin, Protocol, ProtocolPlugin, Serve, ShutdownSignal,
};

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

/// Must never start because it is registered after the failing component.
#[component]
struct NeverStartedComponent {
    #[default]
    started: AtomicUsize,
    #[default]
    stopped: AtomicUsize,
}

/// Has more than one startup hook: cleanup is still required when a later hook fails.
#[component]
struct PartiallyStartedComponent {
    #[default]
    startups: AtomicUsize,
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

impl NeverStartedComponent {
    fn started(&self) -> usize {
        self.started.load(Ordering::SeqCst)
    }

    fn stopped(&self) -> usize {
        self.stopped.load(Ordering::SeqCst)
    }
}

impl PartiallyStartedComponent {
    fn startups(&self) -> usize {
        self.startups.load(Ordering::SeqCst)
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

#[methods]
impl NeverStartedComponent {
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
impl PartiallyStartedComponent {
    #[hook(Startup)]
    async fn first_startup(&self) -> overseerd::daemon::Result<()> {
        self.startups.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }

    #[hook(Startup)]
    async fn second_startup_fails(&self) -> overseerd::daemon::Result<()> {
        self.startups.fetch_add(1, Ordering::SeqCst);

        Err(overseerd::daemon::Error::MissingComponent(
            "second startup rejected",
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
async fn startup_failure_stops_later_hooks_and_only_shuts_down_started_components() {
    let daemon = App::builder("startup-failure-cleanup-test")
        .config_source(ConfigManager::<Toml>::empty())
        .component::<LifecycleComponent>()
        .component::<FailingStartupComponent>()
        .component::<NeverStartedComponent>()
        .build()
        .await
        .expect("daemon builds");

    let component = daemon
        .container()
        .get::<FailingStartupComponent>()
        .expect("component built");
    let started = daemon
        .container()
        .get::<LifecycleComponent>()
        .expect("started component built");
    let never = daemon
        .container()
        .get::<NeverStartedComponent>()
        .expect("later component built");

    let result = daemon.run().await;

    assert!(result.is_err(), "startup failure is returned");
    assert_eq!(started.started(), 1, "first registered component started");
    assert_eq!(started.stopped(), 1, "started component was shut down");
    assert_eq!(component.started(), 1, "startup hook ran once");
    assert_eq!(component.stopped(), 0, "failed startup was not shut down");
    assert_eq!(never.started(), 0, "later startup hook never ran");
    assert_eq!(
        never.stopped(),
        0,
        "never-started component was not shut down"
    );
}

#[tokio::test]
async fn later_startup_failure_preserves_cleanup_for_an_already_started_component() {
    let app = App::builder("partial-startup-cleanup-test")
        .config_source(ConfigManager::<Toml>::empty())
        .component::<PartiallyStartedComponent>()
        .build()
        .await
        .expect("app builds");
    let component = app
        .container()
        .get::<PartiallyStartedComponent>()
        .expect("component built");

    let result = app.run().await;

    assert!(result.is_err());
    assert_eq!(component.startups(), 2, "both startup hooks ran in order");
    assert_eq!(
        component.stopped(),
        1,
        "the earlier successful startup kept the component eligible for cleanup"
    );
}

#[derive(Default)]
struct PanickingPlugin;

struct PanickingProtocol;

impl Plugin for PanickingPlugin {
    fn register(&self, _registry: &mut AppRegistry) {}
}

impl ProtocolPlugin for PanickingPlugin {
    type Protocol = PanickingProtocol;
    type Error = overseerd_app::Error;

    const SCOPES: &'static [&'static dyn overseerd::Scope] = &[];

    fn build(self, _runtime: &AppRuntime) -> Result<Self::Protocol, Self::Error> {
        Ok(PanickingProtocol)
    }
}

impl Protocol for PanickingProtocol {
    type Error = overseerd_app::Error;
}

impl Serve<()> for PanickingProtocol {
    async fn serve(
        self,
        _runtime: AppRuntime,
        _shutdown: ShutdownSignal,
        _endpoint: (),
    ) -> Result<(), Self::Error> {
        panic!("protocol panic")
    }
}

#[tokio::test]
async fn protocol_panic_still_runs_shutdown_hooks() {
    let app = overseerd_app::App::<PanickingPlugin>::builder("panic-cleanup-test")
        .config_source(ConfigManager::<Toml>::empty())
        .component::<LifecycleComponent>()
        .build()
        .await
        .expect("app builds");
    let component = app
        .container()
        .get::<LifecycleComponent>()
        .expect("component built");

    let result = std::panic::AssertUnwindSafe(app.serve(()))
        .catch_unwind()
        .await;

    assert!(result.is_err(), "protocol panic is resumed after cleanup");
    assert_eq!(component.started(), 1);
    assert_eq!(component.stopped(), 1, "shutdown ran before panic resumed");
}
