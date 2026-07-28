#[cfg(feature = "tooling")]
use overseerd::component;
use overseerd::{
    App, AppBuilder, AppRegistry, AppRuntime, BootstrapContext, ExecutionMode, PreparedProtocol,
    ProtocolDefinition, ProtocolRuntime, app,
};
#[cfg(any(feature = "cli", feature = "tooling"))]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "cli")]
static HELP_SETUP_CALLS: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "tooling")]
static TOOLING_SETUP_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "tooling")]
static TOOLING_CONFIGURE_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "tooling")]
static TOOLING_BEFORE_BUILD_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "tooling")]
static TOOLING_COMPONENT_BUILDS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "tooling")]
static TOOLING_PROTOCOL_PREPARES: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "tooling")]
static TOOLING_PROTOCOL_BUILDS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "tooling")]
static TOOLING_AFTER_BUILD_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "tooling")]
static TOOLING_SERVE_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "tooling")]
static TOOLING_PLUGIN_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "tooling")]
static TOOLING_PLUGIN_CONTRIBUTIONS: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(feature = "tooling", feature = "cli"))]
static TOOLING_PLUGIN_CLI_CALLS: AtomicUsize = AtomicUsize::new(0);

/// Test protocol definition selected by the named application host.
#[derive(Default)]
pub struct TestProtocol;

impl ProtocolDefinition for TestProtocol {
    type Prepared = PreparedTestProtocol;
    type Error = overseerd_app::Error;

    const ID: overseerd::ProtocolId =
        overseerd::namespaced_id!(overseerd::ProtocolId, "test/app-definition");
    const SCOPE_TOPOLOGY: overseerd::ScopeTopology = overseerd::ScopeTopology::empty();

    fn register(&self, _registry: &mut AppRegistry) {}

    fn prepare(
        self,
        _context: &overseerd::ValidationContext<'_>,
    ) -> Result<Self::Prepared, Self::Error> {
        Ok(PreparedTestProtocol)
    }
}

/// Prepared test protocol used only to type-check host expansion.
pub struct PreparedTestProtocol;

/// Built protocol runtime used only to type-check host expansion.
pub struct TestRuntime;

impl PreparedProtocol for PreparedTestProtocol {
    type Runtime = TestRuntime;
    type Error = overseerd_app::Error;

    fn build(self, _runtime: &AppRuntime) -> Result<Self::Runtime, Self::Error> {
        Ok(TestRuntime)
    }
}

impl ProtocolRuntime for TestRuntime {
    type Error = overseerd_app::Error;
}

app! {
    pub app TestApplication {
        name: "named-app-test",
        protocol: TestProtocol,
    }
}

macro_rules! forwarded_app {
    ($protocol:ty, $setup:path, $serve:path) => {
        app! {
            app ForwardedApplication {
                name: "forwarded-app-test",
                protocol: $protocol,
                cli: {
                    config: true,
                    profile: true,
                    log: true,
                    log_format: true,
                    color: true,
                    serve: true,
                },
                setup = $setup,
                serve = $serve,
            }
        }
    };
}

async fn forwarded_setup(context: BootstrapContext) -> std::io::Result<BootstrapContext> {
    Ok(context)
}

async fn forwarded_serve(
    _context: BootstrapContext,
    _app: App<TestProtocol>,
) -> std::io::Result<()> {
    Ok(())
}

forwarded_app!(TestProtocol, forwarded_setup, forwarded_serve);

#[cfg(feature = "cli")]
async fn help_setup(context: BootstrapContext) -> std::io::Result<BootstrapContext> {
    HELP_SETUP_CALLS.fetch_add(1, Ordering::SeqCst);

    Ok(context)
}

#[cfg(feature = "cli")]
async fn help_serve(_context: BootstrapContext, _app: App<TestProtocol>) -> std::io::Result<()> {
    Ok(())
}

#[cfg(feature = "cli")]
app! {
    app HelpApplication {
        name: "help-app-test",
        protocol: TestProtocol,
        setup = help_setup,
        serve = help_serve,
    }
}

async fn setup_lifecycle(mut context: BootstrapContext) -> std::io::Result<BootstrapContext> {
    context.insert(vec!["setup"]);

    Ok(context)
}

async fn before_lifecycle(
    context: &mut BootstrapContext,
    builder: AppBuilder<TestProtocol>,
) -> std::io::Result<AppBuilder<TestProtocol>> {
    context
        .get_mut::<Vec<&'static str>>()
        .expect("lifecycle events exist")
        .push("before_build");

    Ok(builder)
}

async fn serve_lifecycle(
    context: BootstrapContext,
    _app: App<TestProtocol>,
) -> std::io::Result<()> {
    assert_eq!(
        context.get::<Vec<&'static str>>(),
        Some(&vec!["setup", "configure", "before_build", "after_build"])
    );

    Ok(())
}

app! {
    app LifecycleApplication {
        name: "lifecycle-app-test",
        protocol: TestProtocol,
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

async fn failing_setup(_context: BootstrapContext) -> std::io::Result<BootstrapContext> {
    Err(std::io::Error::other(
        "setup failed with secret api-token=probe-secret",
    ))
}

app! {
    app FailingLifecycleApplication {
        name: "failing-lifecycle-app-test",
        protocol: TestProtocol,
        setup = failing_setup,
    }
}

app! {
    app DirectoryConfigApplication {
        name: "named-directory-config-test",
        protocol: TestProtocol,
        managers: {
            directories: { root: std::env::temp_dir() },
            config: {},
        },
    }
}

#[cfg(feature = "tooling")]
/// Component whose factory marks accidental tooling construction.
#[component(factory = build_tooling_component)]
pub struct ToolingComponent;

#[cfg(feature = "tooling")]
async fn build_tooling_component() -> ToolingComponent {
    TOOLING_COMPONENT_BUILDS.fetch_add(1, Ordering::SeqCst);

    ToolingComponent
}

#[cfg(feature = "tooling")]
/// Plugin proving the target probe resolves and lowers the retained early catalog once.
pub struct ToolingPlugin;

#[cfg(feature = "tooling")]
impl Default for ToolingPlugin {
    fn default() -> Self {
        TOOLING_PLUGIN_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);

        Self
    }
}

#[cfg(feature = "tooling")]
impl overseerd::Plugin for ToolingPlugin {
    const ID: overseerd::PluginId =
        overseerd::namespaced_id!(overseerd::PluginId, "test/tooling-entry");

    fn contribute(self, _contributions: &mut overseerd::PluginContributions) {
        TOOLING_PLUGIN_CONTRIBUTIONS.fetch_add(1, Ordering::SeqCst);
    }

    #[cfg(feature = "cli")]
    fn cli(&self, cli: &mut overseerd::PluginCliRegistrar) {
        TOOLING_PLUGIN_CLI_CALLS.fetch_add(1, Ordering::SeqCst);
        cli.args::<ToolingPluginArgs>(overseerd::namespaced_id!(
            overseerd::ContributionId,
            "test/tooling-entry-args"
        ));
    }
}

#[cfg(all(feature = "tooling", feature = "cli"))]
/// Parser arguments used to prove one catalog composes tooling metadata once.
#[derive(clap::Args)]
pub struct ToolingPluginArgs {
    /// Enables the tooling fixture marker.
    #[arg(long)]
    tooling_fixture: bool,
}

#[cfg(feature = "tooling")]
/// Protocol definition used to prove tooling preparation stops before runtime construction.
#[derive(Default)]
pub struct ToolingProtocol;

#[cfg(feature = "tooling")]
impl ProtocolDefinition for ToolingProtocol {
    type Prepared = PreparedToolingProtocol;
    type Error = overseerd_app::Error;

    const ID: overseerd::ProtocolId =
        overseerd::namespaced_id!(overseerd::ProtocolId, "test/tooling-entry");
    const SCOPE_TOPOLOGY: overseerd::ScopeTopology = overseerd::ScopeTopology::empty();

    fn register(&self, _registry: &mut AppRegistry) {}

    fn prepare(
        self,
        _context: &overseerd::ValidationContext<'_>,
    ) -> Result<Self::Prepared, Self::Error> {
        TOOLING_PROTOCOL_PREPARES.fetch_add(1, Ordering::SeqCst);

        Ok(PreparedToolingProtocol)
    }
}

#[cfg(feature = "tooling")]
/// Prepared protocol marker for the generated tooling entry proof.
pub struct PreparedToolingProtocol;

#[cfg(feature = "tooling")]
/// Runtime protocol marker that tooling must never construct.
pub struct ToolingRuntime;

#[cfg(feature = "tooling")]
impl PreparedProtocol for PreparedToolingProtocol {
    type Runtime = ToolingRuntime;
    type Error = overseerd_app::Error;

    fn build(self, _runtime: &AppRuntime) -> Result<Self::Runtime, Self::Error> {
        TOOLING_PROTOCOL_BUILDS.fetch_add(1, Ordering::SeqCst);

        Ok(ToolingRuntime)
    }
}

#[cfg(feature = "tooling")]
impl ProtocolRuntime for ToolingRuntime {
    type Error = overseerd_app::Error;
}

#[cfg(feature = "tooling")]
async fn tooling_setup(context: BootstrapContext) -> std::io::Result<BootstrapContext> {
    assert!(context.mode().is_tooling());
    TOOLING_SETUP_CALLS.fetch_add(1, Ordering::SeqCst);
    println!("application setup stdout is not probe transport");

    Ok(context)
}

#[cfg(feature = "tooling")]
async fn panicking_tooling_setup(_context: BootstrapContext) -> std::io::Result<BootstrapContext> {
    panic!("panic payload contains probe-secret");
}

#[cfg(feature = "tooling")]
app! {
    pub app PanickingToolingApplication {
        name: "panicking-tooling-entry-test",
        protocol: TestProtocol,
        setup = panicking_tooling_setup,
    }
}

#[cfg(feature = "tooling")]
async fn tooling_configure(
    context: &mut BootstrapContext,
    builder: AppBuilder<ToolingProtocol>,
) -> std::io::Result<AppBuilder<ToolingProtocol>> {
    assert!(context.mode().is_tooling());
    TOOLING_CONFIGURE_CALLS.fetch_add(1, Ordering::SeqCst);

    Ok(builder)
}

#[cfg(feature = "tooling")]
async fn tooling_before_build(
    context: &mut BootstrapContext,
    builder: AppBuilder<ToolingProtocol>,
) -> std::io::Result<AppBuilder<ToolingProtocol>> {
    assert!(context.mode().is_tooling());
    TOOLING_BEFORE_BUILD_CALLS.fetch_add(1, Ordering::SeqCst);

    Ok(builder)
}

#[cfg(feature = "tooling")]
async fn tooling_after_build(
    _context: &mut BootstrapContext,
    app: App<ToolingProtocol>,
) -> std::io::Result<App<ToolingProtocol>> {
    TOOLING_AFTER_BUILD_CALLS.fetch_add(1, Ordering::SeqCst);

    Ok(app)
}

#[cfg(feature = "tooling")]
async fn tooling_serve(
    _context: BootstrapContext,
    _app: App<ToolingProtocol>,
) -> std::io::Result<()> {
    TOOLING_SERVE_CALLS.fetch_add(1, Ordering::SeqCst);

    Ok(())
}

#[cfg(feature = "tooling")]
app! {
    app ToolingApplication {
        name: "tooling-entry-test",
        protocol: ToolingProtocol,
        components: [ToolingComponent],
        managers: {
            directories: { root: std::env::temp_dir() },
            config: {},
        },
        plugins: [ToolingPlugin],
        setup = tooling_setup,
        configure = tooling_configure,
        before_build = tooling_before_build,
        after_build = tooling_after_build,
        serve = tooling_serve,
    }
}

fn assert_builder(_builder: AppBuilder<TestProtocol>) {}

#[test]
fn named_app_creates_independent_typed_builders() {
    assert_builder(TestApplication::builder().expect("first builder"));
    assert_builder(TestApplication::builder().expect("second builder"));
}

#[test]
fn macro_rules_forwarded_app_preserves_generated_host_hygiene() {
    assert_builder(ForwardedApplication::builder().expect("forwarded builder"));
    let _ = forwarded_setup;
    let _ = forwarded_serve;
}

#[test]
fn named_app_loads_directory_backed_config_fallibly() {
    assert_builder(DirectoryConfigApplication::builder().expect("directory config loads"));
}

#[tokio::test]
async fn named_app_runs_lifecycle_phases_in_order() {
    let setup = LifecycleApplication::new(ExecutionMode::Run)
        .setup()
        .await
        .expect("application sets up");

    assert_eq!(
        setup.context().get::<Vec<&'static str>>(),
        Some(&vec!["setup"])
    );

    let prepared = setup.prepare().await.expect("application prepares");

    assert!(prepared.context().mode().is_run());
    assert_eq!(
        prepared.context().get::<Vec<&'static str>>(),
        Some(&vec!["setup", "configure", "before_build"])
    );

    let built = prepared.build().await.expect("application builds");

    assert_eq!(
        built.context().get::<Vec<&'static str>>(),
        Some(&vec!["setup", "configure", "before_build", "after_build"])
    );
    assert_eq!(built.app().name, "lifecycle-app-test");

    built.serve().await.expect("serve phase runs");
}

#[tokio::test]
async fn named_app_explicitly_fast_forwards_lifecycle_stages() {
    let prepared = LifecycleApplication::new(ExecutionMode::Tooling)
        .prepare()
        .await
        .expect("application fast-forwards to pre-build");

    assert!(prepared.context().mode().is_tooling());
    assert_eq!(
        prepared.context().get::<Vec<&'static str>>(),
        Some(&vec!["setup", "configure", "before_build"])
    );

    let (_, prepared) = prepared.into_parts();
    let _: App<TestProtocol> = prepared.build().await.expect("prepared app builds");

    let built = LifecycleApplication::new(ExecutionMode::Run)
        .build()
        .await
        .expect("application fast-forwards to built");

    assert_eq!(
        built.context().get::<Vec<&'static str>>(),
        Some(&vec!["setup", "configure", "before_build", "after_build"])
    );
}

#[tokio::test]
async fn named_app_tags_lifecycle_errors_with_their_phase() {
    let result = FailingLifecycleApplication::new(ExecutionMode::Run)
        .prepare()
        .await;
    let error = match result {
        Ok(_) => panic!("setup phase unexpectedly succeeded"),
        Err(error) => error,
    };

    assert_eq!(error.phase(), overseerd::LifecyclePhase::Setup);
    assert_eq!(
        error.to_string(),
        "setup phase failed: setup failed with secret api-token=probe-secret"
    );
}

#[tokio::test]
async fn named_app_rejects_component_construction_in_tooling_mode() {
    let prepared = LifecycleApplication::new(ExecutionMode::Tooling)
        .prepare()
        .await
        .expect("tooling lifecycle prepares and validates");

    assert_eq!(
        prepared.context().get::<Vec<&'static str>>(),
        Some(&vec!["setup", "configure", "before_build"])
    );

    let error = match prepared.build().await {
        Ok(_) => panic!("tooling mode unexpectedly constructed the application"),
        Err(error) => error,
    };

    assert_eq!(error.phase(), overseerd::LifecyclePhase::Build);
    assert_eq!(
        error.to_string(),
        "build phase failed: tooling mode cannot construct application components or protocols"
    );
}

#[tokio::test]
#[cfg(feature = "tooling")]
async fn generated_tooling_entry_prepares_real_target_without_constructing_runtime() {
    for counter in [
        &TOOLING_SETUP_CALLS,
        &TOOLING_CONFIGURE_CALLS,
        &TOOLING_BEFORE_BUILD_CALLS,
        &TOOLING_COMPONENT_BUILDS,
        &TOOLING_PROTOCOL_PREPARES,
        &TOOLING_PROTOCOL_BUILDS,
        &TOOLING_AFTER_BUILD_CALLS,
        &TOOLING_SERVE_CALLS,
        &TOOLING_PLUGIN_CONSTRUCTIONS,
        &TOOLING_PLUGIN_CONTRIBUTIONS,
    ] {
        counter.store(0, Ordering::SeqCst);
    }
    #[cfg(feature = "cli")]
    TOOLING_PLUGIN_CLI_CALLS.store(0, Ordering::SeqCst);

    let envelope = ToolingApplication::tooling_probe(tooling_target("thin-tooling-bin"))
        .await
        .expect("generated declaration identity validates");
    let overseerd::tooling::ProbeOutcome::Success { document } = envelope.outcome else {
        panic!("generated tooling probe unexpectedly failed");
    };

    assert_eq!(TOOLING_SETUP_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(TOOLING_CONFIGURE_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(TOOLING_BEFORE_BUILD_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(TOOLING_PROTOCOL_PREPARES.load(Ordering::SeqCst), 1);
    assert_eq!(TOOLING_PLUGIN_CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    assert_eq!(TOOLING_PLUGIN_CONTRIBUTIONS.load(Ordering::SeqCst), 1);
    #[cfg(feature = "cli")]
    assert_eq!(TOOLING_PLUGIN_CLI_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(TOOLING_COMPONENT_BUILDS.load(Ordering::SeqCst), 0);
    assert_eq!(TOOLING_PROTOCOL_BUILDS.load(Ordering::SeqCst), 0);
    assert_eq!(TOOLING_AFTER_BUILD_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(TOOLING_SERVE_CALLS.load(Ordering::SeqCst), 0);

    assert_eq!(document.identity.application, "tooling-entry-test");
    assert_eq!(
        document
            .identity
            .package
            .as_ref()
            .map(|package| package.name.as_str()),
        Some("overseerd")
    );
    assert_eq!(
        document
            .identity
            .binary
            .as_ref()
            .map(|binary| binary.name.as_str()),
        Some("thin-tooling-bin")
    );
    assert!(
        document
            .identity
            .source
            .as_ref()
            .is_some_and(|source| source_path_ends_with(
                &source.file,
                &["tests", "app_definition.rs"]
            ))
    );
    assert!(document.resources.iter().any(|resource| {
        resource.id == "component:toolingcomponent" || resource.name == "ToolingComponent"
    }));
    assert!(document.resources.iter().any(|resource| {
        resource.id == "plugin:test/tooling-entry" || resource.name.contains("tooling-entry")
    }));
    #[cfg(feature = "cli")]
    assert!(document.cli.as_ref().is_some_and(|cli| {
        cli.root
            .arguments
            .iter()
            .any(|argument| argument.id == "tooling_fixture")
    }));
}

#[tokio::test]
#[cfg(feature = "tooling")]
async fn generated_tooling_entry_returns_typed_lifecycle_failure_envelope() {
    let envelope = FailingLifecycleApplication::tooling_probe(tooling_target("failure-bin"))
        .await
        .expect("generated declaration identity validates");
    let overseerd::tooling::ProbeOutcome::Failure { failure } = &envelope.outcome else {
        panic!("failing application unexpectedly produced a document");
    };
    let diagnostic = &failure.diagnostics[0];

    assert_eq!(envelope.identity.application, "failing-lifecycle-app-test");
    assert_eq!(diagnostic.code, "overseerd/tooling-setup");
    assert_eq!(failure.phase.as_deref(), Some("setup"));
    assert_eq!(
        diagnostic.message,
        "Application setup failed during the tooling probe."
    );
    assert!(!diagnostic.message.contains("probe-secret"));
    assert!(
        !envelope
            .to_json()
            .expect("failure serializes")
            .contains("probe-secret")
    );
}

#[tokio::test]
#[cfg(feature = "tooling")]
async fn tooling_response_file_is_pure_json_when_application_writes_stdout() {
    let envelope = ToolingApplication::tooling_probe(tooling_target("response-file-bin"))
        .await
        .expect("generated declaration identity validates");
    let path = probe_output_path("stdout-purity");

    overseerd_app::tooling::emit_probe_envelope(&path, &envelope)
        .expect("response file is emitted");

    let json = std::fs::read_to_string(&path).expect("response file is readable");
    let decoded = overseerd::tooling::ProbeEnvelope::from_json(json.trim_end())
        .expect("response contains only one valid envelope");

    std::fs::remove_file(path).expect("response fixture is removed");
    assert!(decoded.is_success());
}

#[tokio::test]
#[cfg(feature = "tooling")]
async fn library_defined_application_uses_explicit_thin_binary_identity() {
    let envelope = ToolingApplication::tooling_probe(tooling_target("selected-thin-binary"))
        .await
        .expect("generated declaration identity validates");

    assert_eq!(
        envelope
            .identity
            .binary
            .as_ref()
            .map(|binary| binary.name.as_str()),
        Some("selected-thin-binary")
    );
    assert_ne!(
        envelope
            .identity
            .binary
            .as_ref()
            .map(|binary| binary.name.as_str()),
        Some(env!("CARGO_CRATE_NAME"))
    );
    assert!(
        envelope
            .identity
            .source
            .as_ref()
            .is_some_and(|source| source_path_ends_with(
                &source.file,
                &["tests", "app_definition.rs"]
            ))
    );
}

#[cfg(feature = "tooling")]
fn source_path_ends_with(source: &str, suffix: &[&str]) -> bool {
    let components = std::path::Path::new(source)
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();

    components.ends_with(suffix)
}

#[tokio::test]
#[cfg(feature = "tooling")]
async fn generated_tooling_entry_sanitizes_panics() {
    let envelope = PanickingToolingApplication::tooling_probe(tooling_target("panic-bin"))
        .await
        .expect("generated declaration identity validates");
    let overseerd::tooling::ProbeOutcome::Failure { failure } = &envelope.outcome else {
        panic!("panicking application unexpectedly produced a document");
    };
    let diagnostic = &failure.diagnostics[0];

    assert_eq!(diagnostic.code, "overseerd/tooling-panic");
    assert_eq!(
        diagnostic.message,
        "The tooling probe panicked while preparing the application."
    );
    assert!(
        !envelope
            .to_json()
            .expect("panic serializes")
            .contains("probe-secret")
    );
}

#[cfg(feature = "tooling")]
fn tooling_target(binary: &str) -> overseerd::tooling::ProbeTargetIdentity {
    overseerd::tooling::ProbeTargetIdentity::new(
        overseerd::tooling::PackageIdentity {
            name: String::from("overseerd"),
            version: Some(String::from(env!("CARGO_PKG_VERSION"))),
            manifest_path: Some(format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR"))),
        },
        overseerd::tooling::BinaryTargetIdentity {
            name: binary.to_string(),
        },
    )
    .expect("test target identity is valid")
}

#[cfg(feature = "tooling")]
fn probe_output_path(label: &str) -> std::path::PathBuf {
    static NEXT_PATH: AtomicUsize = AtomicUsize::new(0);

    let ordinal = NEXT_PATH.fetch_add(1, Ordering::Relaxed);

    std::env::temp_dir().join(format!(
        "overseerd-probe-{label}-{}-{ordinal}.json",
        std::process::id()
    ))
}

#[test]
#[cfg(feature = "cli")]
fn generated_cli_exposes_native_clap_types() {
    use clap::{CommandFactory as _, Parser as _};

    let command = LifecycleApplicationCli::command();
    let default_cli = LifecycleApplicationCli::try_parse_from(["lifecycle-app-test"])
        .expect("default command parses");
    let serve_cli = LifecycleApplicationCli::try_parse_from(["lifecycle-app-test", "serve"])
        .expect("serve command parses");

    assert_eq!(command.get_name(), "lifecycle-app-test");
    assert!(default_cli.command.is_none());
    assert!(matches!(
        serve_cli.command,
        Some(LifecycleApplicationCommand::Serve)
    ));
}

#[tokio::test]
#[cfg(feature = "cli")]
async fn generated_cli_help_and_version_do_not_run_setup() {
    HELP_SETUP_CALLS.store(0, Ordering::SeqCst);

    for (argument, expected) in [
        ("--help", clap::error::ErrorKind::DisplayHelp),
        ("--version", clap::error::ErrorKind::DisplayVersion),
    ] {
        let error = HelpApplication::run_with(["help-app-test", argument])
            .await
            .expect_err("early output is returned to the caller");

        assert!(matches!(error, overseerd::CliError::Clap(error) if error.kind() == expected));
    }

    assert_eq!(HELP_SETUP_CALLS.load(Ordering::SeqCst), 0);
}
