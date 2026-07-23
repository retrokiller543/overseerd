use overseerd::{
    App, AppBuilder, AppRegistry, AppRuntime, BootstrapContext, ExecutionMode, Plugin, Protocol,
    ProtocolPlugin, app,
};

/// Test protocol accumulated by the named application host.
#[derive(Default)]
pub struct TestPlugin;

impl Plugin for TestPlugin {
    fn register(&self, _registry: &mut AppRegistry) {}
}

/// Built protocol used only to type-check host expansion.
pub struct TestProtocol;

impl Protocol for TestProtocol {
    type Error = overseerd_app::Error;
}

impl ProtocolPlugin for TestPlugin {
    type Protocol = TestProtocol;
    type Error = overseerd_app::Error;

    const SCOPES: &'static [&'static dyn overseerd::Scope] = &[];

    fn build(self, _runtime: &AppRuntime) -> Result<Self::Protocol, Self::Error> {
        Ok(TestProtocol)
    }
}

app! {
    pub app TestApplication {
        name: "named-app-test",
        protocol: TestPlugin,
    }
}

async fn setup_lifecycle(mode: ExecutionMode) -> std::io::Result<BootstrapContext> {
    let mut context = BootstrapContext::new(mode);

    context.insert(vec!["setup"]);

    Ok(context)
}

async fn before_lifecycle(
    context: &mut BootstrapContext,
    builder: AppBuilder<TestPlugin>,
) -> std::io::Result<AppBuilder<TestPlugin>> {
    context
        .get_mut::<Vec<&'static str>>()
        .expect("lifecycle events exist")
        .push("before_build");

    Ok(builder)
}

async fn serve_lifecycle(context: BootstrapContext, _app: App<TestPlugin>) -> std::io::Result<()> {
    assert_eq!(
        context.get::<Vec<&'static str>>(),
        Some(&vec!["setup", "configure", "before_build", "after_build"])
    );

    Ok(())
}

app! {
    app LifecycleApplication {
        name: "lifecycle-app-test",
        protocol: TestPlugin,
        setup = setup_lifecycle,
        configure(builder, context) {
            builder
                .get_mut::<Vec<&'static str>>()
                .expect("lifecycle events exist")
                .push("configure");

            Ok::<_, std::io::Error>(context)
        },
        before_build = before_lifecycle,
        after_build(context, app) {
            context
                .get_mut::<Vec<&'static str>>()
                .expect("lifecycle events exist")
                .push("after_build");

            Ok::<_, std::io::Error>(app)
        },
        serve = serve_lifecycle,
    }
}

async fn failing_setup(_mode: ExecutionMode) -> std::io::Result<BootstrapContext> {
    Err(std::io::Error::other("setup failed"))
}

app! {
    app FailingLifecycleApplication {
        name: "failing-lifecycle-app-test",
        protocol: TestPlugin,
        setup = failing_setup,
    }
}

app! {
    app DirectoryConfigApplication {
        name: "named-directory-config-test",
        protocol: TestPlugin,
        managers: {
            directories: { root: std::env::temp_dir() },
            config: {},
        },
    }
}

fn assert_builder(_builder: AppBuilder<TestPlugin>) {}

#[test]
fn named_app_creates_independent_typed_builders() {
    assert_builder(TestApplication::builder().expect("first builder"));
    assert_builder(TestApplication::builder().expect("second builder"));
}

#[test]
fn named_app_loads_directory_backed_config_fallibly() {
    assert_builder(DirectoryConfigApplication::builder().expect("directory config loads"));
}

#[tokio::test]
async fn named_app_runs_lifecycle_phases_in_order() {
    let (context, prepared) = LifecycleApplication::prepare(ExecutionMode::Tooling)
        .await
        .expect("application prepares");

    assert!(context.mode().is_tooling());
    assert_eq!(
        context.get::<Vec<&'static str>>(),
        Some(&vec!["setup", "configure", "before_build"])
    );

    let (context, app) = LifecycleApplication::build(ExecutionMode::Run)
        .await
        .expect("application builds");

    assert_eq!(
        context.get::<Vec<&'static str>>(),
        Some(&vec!["setup", "configure", "before_build", "after_build"])
    );
    assert_eq!(app.name, "lifecycle-app-test");

    let _: App<TestPlugin> = prepared.build().await.expect("prepared app builds");

    LifecycleApplication::serve_with(context, app)
        .await
        .expect("serve phase runs");
}

#[tokio::test]
async fn named_app_tags_lifecycle_errors_with_their_phase() {
    let result = FailingLifecycleApplication::prepare(ExecutionMode::Run).await;
    let error = match result {
        Ok(_) => panic!("setup phase unexpectedly succeeded"),
        Err(error) => error,
    };

    assert_eq!(error.phase(), overseerd::LifecyclePhase::Setup);
    assert_eq!(error.to_string(), "setup phase failed: setup failed");
}

#[tokio::test]
async fn named_app_rejects_component_construction_in_tooling_mode() {
    let result = LifecycleApplication::build(ExecutionMode::Tooling).await;
    let error = match result {
        Ok(_) => panic!("tooling mode unexpectedly constructed the application"),
        Err(error) => error,
    };

    assert_eq!(error.phase(), overseerd::LifecyclePhase::Build);
    assert_eq!(
        error.to_string(),
        "build phase failed: tooling mode cannot construct application components or protocols"
    );
}
