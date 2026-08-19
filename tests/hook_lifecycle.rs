//! Proof of the builtin `Startup`/`Shutdown` lifecycle hooks: `run()` fires `Startup` before
//! waiting and `Shutdown` once a graceful stop is triggered.
#![cfg(feature = "daemon")]
#![allow(dead_code)]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::FutureExt;
use upwell::config::Toml;
use upwell::daemon::App;
use upwell::{ConfigManager, Shutdown, Startup, component, methods};
use upwell_app::{
    AppRegistry, AppRuntime, PreparedProtocol, ProtocolDefinition, ProtocolRuntime, ScopeTopology,
    Serve, ShutdownSignal, ValidationContext,
};

use common::{AbortOnDropTask, deadline};

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

#[derive(Default)]
struct PanicProtocol;

struct PreparedPanicProtocol;

struct PanicRuntime;

#[derive(Clone, Copy)]
enum PanicPhase {
    Construction,
    Poll,
}

impl ProtocolDefinition for PanicProtocol {
    type Prepared = PreparedPanicProtocol;
    type Error = upwell::daemon::Error;

    const ID: upwell_app::ProtocolId =
        upwell::namespaced_id!(upwell_app::ProtocolId, "test/panic-cleanup");
    const SCOPE_TOPOLOGY: ScopeTopology = ScopeTopology::empty();

    fn register(&self, _registry: &mut AppRegistry) {}

    fn prepare(self, _context: &ValidationContext<'_>) -> Result<Self::Prepared, Self::Error> {
        Ok(PreparedPanicProtocol)
    }
}

impl PreparedProtocol for PreparedPanicProtocol {
    type Runtime = PanicRuntime;
    type Error = upwell::daemon::Error;

    fn build(self, _runtime: &AppRuntime) -> Result<Self::Runtime, Self::Error> {
        Ok(PanicRuntime)
    }
}

impl ProtocolRuntime for PanicRuntime {
    type Error = upwell::daemon::Error;
}

impl Serve<PanicPhase> for PanicRuntime {
    fn serve(
        self,
        _runtime: AppRuntime,
        _shutdown: ShutdownSignal,
        phase: PanicPhase,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        if matches!(phase, PanicPhase::Construction) {
            panic!("serve future construction panic");
        }

        async move { panic!("serve future polling panic") }
    }
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
    async fn on_start(&self) -> upwell::daemon::Result<()> {
        self.started.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }

    #[hook(Shutdown)]
    async fn on_stop(&self) -> upwell::daemon::Result<()> {
        self.stopped.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }
}

#[methods]
impl FailingStartupComponent {
    #[hook(Startup)]
    async fn on_start(&self) -> upwell::daemon::Result<()> {
        self.started.fetch_add(1, Ordering::SeqCst);

        Err(upwell::daemon::Error::MissingComponent("startup rejected"))
    }

    #[hook(Shutdown)]
    async fn on_stop(&self) -> upwell::daemon::Result<()> {
        self.stopped.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }
}

#[methods]
impl NeverStartedComponent {
    #[hook(Startup)]
    async fn on_start(&self) -> upwell::daemon::Result<()> {
        self.started.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }

    #[hook(Shutdown)]
    async fn on_stop(&self) -> upwell::daemon::Result<()> {
        self.stopped.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }
}

#[methods]
impl PartiallyStartedComponent {
    #[hook(Startup)]
    async fn first_startup(&self) -> upwell::daemon::Result<()> {
        self.startups.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }

    #[hook(Startup)]
    async fn second_startup_fails(&self) -> upwell::daemon::Result<()> {
        self.startups.fetch_add(1, Ordering::SeqCst);

        Err(upwell::daemon::Error::MissingComponent(
            "second startup rejected",
        ))
    }

    #[hook(Shutdown)]
    async fn on_stop(&self) -> upwell::daemon::Result<()> {
        self.stopped.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }
}

#[tokio::test]
async fn startup_and_shutdown_hooks_fire() {
    let daemon = upwell::daemon::App::builder("lifecycle-test")
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

    let mut task = AbortOnDropTask::spawn("lifecycle daemon", daemon.run());

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
    task.join()
        .await
        .expect("lifecycle daemon stops without error");

    assert_eq!(
        component.stopped(),
        1,
        "shutdown hook fired on graceful stop"
    );
}

#[tokio::test]
async fn serve_panics_still_run_shutdown_hooks() {
    for phase in [PanicPhase::Construction, PanicPhase::Poll] {
        let app = upwell_app::App::<PanicProtocol>::builder("serve-panic-cleanup-test")
            .config_source(ConfigManager::<Toml>::empty())
            .component::<LifecycleComponent>()
            .build()
            .await
            .expect("application builds");
        let component = app
            .container()
            .get::<LifecycleComponent>()
            .expect("lifecycle component built");

        let panic = std::panic::AssertUnwindSafe(app.serve(phase))
            .catch_unwind()
            .await;

        assert!(panic.is_err(), "serve panic is resumed after cleanup");
        assert_eq!(component.started(), 1);
        assert_eq!(component.stopped(), 1, "shutdown hook ran after panic");
    }
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

    let result = deadline("startup failure cleanup", daemon.run()).await;

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

    let result = deadline("partial startup cleanup", app.run()).await;

    assert!(result.is_err());
    assert_eq!(component.startups(), 2, "both startup hooks ran in order");
    assert_eq!(
        component.stopped(),
        1,
        "the earlier successful startup kept the component eligible for cleanup"
    );
}

#[derive(Default)]
struct PanickingProtocol;

struct PreparedPanickingProtocol;

struct PanickingRuntime;

impl ProtocolDefinition for PanickingProtocol {
    type Prepared = PreparedPanickingProtocol;
    type Error = upwell_app::Error;

    const ID: upwell::ProtocolId = upwell::namespaced_id!(upwell::ProtocolId, "test/panicking");
    const SCOPE_TOPOLOGY: upwell::ScopeTopology = upwell::ScopeTopology::empty();

    fn register(&self, _registry: &mut AppRegistry) {}

    fn prepare(
        self,
        _context: &upwell::ValidationContext<'_>,
    ) -> Result<Self::Prepared, Self::Error> {
        Ok(PreparedPanickingProtocol)
    }
}

impl PreparedProtocol for PreparedPanickingProtocol {
    type Runtime = PanickingRuntime;
    type Error = upwell_app::Error;

    fn build(self, _runtime: &AppRuntime) -> Result<Self::Runtime, Self::Error> {
        Ok(PanickingRuntime)
    }

    #[cfg(feature = "tooling")]
    fn tooling(&self, contributions: &mut upwell_app::ToolingContributions) {
        contributions.display(upwell_app::ResourceDisplay {
            label: Some(String::from("Panicking test protocol")),
            ..Default::default()
        });
    }
}

impl ProtocolRuntime for PanickingRuntime {
    type Error = upwell_app::Error;
}

impl Serve<()> for PanickingRuntime {
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
    let app = upwell_app::App::<PanickingProtocol>::builder("panic-cleanup-test")
        .config_source(ConfigManager::<Toml>::empty())
        .component::<LifecycleComponent>()
        .build()
        .await
        .expect("app builds");
    let component = app
        .container()
        .get::<LifecycleComponent>()
        .expect("component built");

    let result = deadline(
        "protocol panic cleanup",
        std::panic::AssertUnwindSafe(app.serve(())).catch_unwind(),
    )
    .await;

    assert!(result.is_err(), "protocol panic is resumed after cleanup");
    assert_eq!(component.started(), 1);
    assert_eq!(component.stopped(), 1, "shutdown ran before panic resumed");
}
