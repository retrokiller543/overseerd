#![cfg(feature = "cli")]

use std::sync::atomic::{AtomicUsize, Ordering};

use clap::{CommandFactory as _, Parser as _};
use overseerd::config::Toml;
use overseerd::{
    App, AppBuilder, AppRegistry, AppRuntime, BootstrapContext, Built, CliCommand, CliError,
    CommandContext, ConfigManager, ContributionId, Plugin, PluginCliCommand, PluginCliRegistrar,
    PluginCommandContext, PluginContributions, PreBuild, PreparedProtocol, ProtocolDefinition,
    ProtocolPluginRegistrar, ProtocolRuntime, Setup, app, component,
};

static SETUP_CALLS: AtomicUsize = AtomicUsize::new(0);
static CONFIGURE_CALLS: AtomicUsize = AtomicUsize::new(0);
static COMPONENT_BUILDS: AtomicUsize = AtomicUsize::new(0);
static PROTOCOL_BUILDS: AtomicUsize = AtomicUsize::new(0);
static AFTER_BUILD_CALLS: AtomicUsize = AtomicUsize::new(0);
static SERVE_CALLS: AtomicUsize = AtomicUsize::new(0);
static PROTOCOL_PLUGIN_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static APPLICATION_PLUGIN_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static PROTOCOL_PLUGIN_CONTRIBUTIONS: AtomicUsize = AtomicUsize::new(0);
static APPLICATION_PLUGIN_CONTRIBUTIONS: AtomicUsize = AtomicUsize::new(0);
static PLUGIN_COMMAND_RUNS: AtomicUsize = AtomicUsize::new(0);
static PROTOCOL_DROPS: AtomicUsize = AtomicUsize::new(0);

/// Global arguments flattened into the generated application parser.
#[derive(clap::Args)]
pub struct OutputArgs {
    /// Output representation used by utility commands.
    #[arg(long, global = true, default_value = "text")]
    format: String,
}

/// Arguments intentionally colliding with framework-owned bootstrap options.
#[derive(clap::Args)]
pub struct CollidingArgs {
    /// Conflicts with the framework's global profile option.
    #[arg(long)]
    profile: Option<String>,
}

/// Plugin arguments intentionally colliding with framework bootstrap options.
#[derive(clap::Args)]
pub struct CollidingPluginArgs {
    /// Conflicts with the framework's global profile option.
    #[arg(long)]
    profile: Option<String>,
}

/// Plugin used to prove provenance-aware parser collisions.
pub struct CollidingCliPlugin;

impl Default for CollidingCliPlugin {
    fn default() -> Self {
        Self
    }
}

impl Plugin for CollidingCliPlugin {
    const ID: overseerd::PluginId =
        overseerd::namespaced_id!(overseerd::PluginId, "test/colliding-cli");

    fn cli(&self, cli: &mut PluginCliRegistrar) {
        cli.args::<CollidingPluginArgs>(overseerd::namespaced_id!(
            ContributionId,
            "test/colliding-args"
        ));
    }

    fn contribute(self, _contributions: &mut PluginContributions) {}
}

/// Global arguments contributed by a protocol-default plugin.
#[derive(clap::Args)]
pub struct ProtocolPluginArgs {
    /// Selects the protocol inspection detail level.
    #[arg(long, global = true, default_value = "summary")]
    protocol_detail: String,
}

/// Setup-only command contributed by an application plugin.
#[derive(clap::Args)]
pub struct PluginInspectCommand;

impl PluginCliCommand for PluginInspectCommand {
    type Phase = Setup;
    type Error = std::io::Error;

    async fn run(&self, context: PluginCommandContext<Self::Phase>) -> Result<(), Self::Error> {
        assert_eq!(
            context
                .require::<ProtocolPluginArgs>()
                .expect("plugin arguments are retained")
                .protocol_detail,
            "full"
        );

        PLUGIN_COMMAND_RUNS.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }
}

/// Native flattened command set contributed by the protocol default.
#[derive(clap::Subcommand)]
pub enum ProtocolPluginCommands {
    /// Reports protocol CLI composition state.
    ProtocolStatus,
}

impl PluginCliCommand for ProtocolPluginCommands {
    type Phase = Setup;
    type Error = std::io::Error;

    async fn run(&self, _context: PluginCommandContext<Self::Phase>) -> Result<(), Self::Error> {
        PLUGIN_COMMAND_RUNS.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }
}

/// Protocol-default plugin contributing args and a native command set.
pub struct ProtocolCliPlugin {
    marker: bool,
}

impl Default for ProtocolCliPlugin {
    fn default() -> Self {
        PROTOCOL_PLUGIN_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);

        Self { marker: true }
    }
}

impl Plugin for ProtocolCliPlugin {
    const ID: overseerd::PluginId =
        overseerd::namespaced_id!(overseerd::PluginId, "test/protocol-cli");

    fn cli(&self, cli: &mut PluginCliRegistrar) {
        cli.args::<ProtocolPluginArgs>(overseerd::namespaced_id!(
            ContributionId,
            "test/protocol-args"
        ));
        cli.commands::<ProtocolPluginCommands>(overseerd::namespaced_id!(
            ContributionId,
            "test/protocol-commands"
        ));
    }

    fn contribute(self, _contributions: &mut PluginContributions) {
        assert!(self.marker);

        PROTOCOL_PLUGIN_CONTRIBUTIONS.fetch_add(1, Ordering::SeqCst);
    }
}

/// Application plugin contributing one leaf command.
pub struct ApplicationCliPlugin;

impl Default for ApplicationCliPlugin {
    fn default() -> Self {
        APPLICATION_PLUGIN_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);

        Self
    }
}

impl Plugin for ApplicationCliPlugin {
    const ID: overseerd::PluginId =
        overseerd::namespaced_id!(overseerd::PluginId, "test/application-cli");

    fn cli(&self, cli: &mut PluginCliRegistrar) {
        cli.command::<PluginInspectCommand>(
            overseerd::namespaced_id!(ContributionId, "test/plugin-inspect-command"),
            "plugin-inspect",
        );
    }

    fn contribute(self, _contributions: &mut PluginContributions) {
        APPLICATION_PLUGIN_CONTRIBUTIONS.fetch_add(1, Ordering::SeqCst);
    }
}

/// Configured command contributed to a plugin-only CLI surface.
#[derive(clap::Args)]
pub struct PluginCatalogCommand;

impl PluginCliCommand for PluginCatalogCommand {
    type Phase = PreBuild;
    type Error = std::io::Error;

    async fn run(&self, context: PluginCommandContext<Self::Phase>) -> Result<(), Self::Error> {
        assert_eq!(context.application_name(), "plugin-only-command-test");
        assert!(
            context
                .registry()
                .resolved_component::<BuildMarker>()
                .expect("validated registry resolves components")
                .is_some()
        );
        assert!(!context.plugin_plan().resolution().plugins().is_empty());

        PLUGIN_COMMAND_RUNS.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }
}

/// Built command proving protocol-neutral plugin DI access.
#[derive(clap::Args)]
pub struct PluginBuildCommand;

impl PluginCliCommand for PluginBuildCommand {
    type Phase = Built;
    type Error = overseerd::DiError;

    async fn run(&self, context: PluginCommandContext<Self::Phase>) -> Result<(), Self::Error> {
        let marker = context.resolve::<std::sync::Arc<BuildMarker>>().await?;

        assert_eq!(context.application_name(), "plugin-only-command-test");
        assert!(!context.plugin_plan().resolution().plugins().is_empty());
        assert_eq!(std::sync::Arc::strong_count(&marker), 2);
        assert_eq!(PROTOCOL_DROPS.load(Ordering::SeqCst), 0);

        PLUGIN_COMMAND_RUNS.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }
}

/// Plugin defining an application's entire CLI surface.
pub struct PluginOnlyCliPlugin;

impl Default for PluginOnlyCliPlugin {
    fn default() -> Self {
        Self
    }
}

impl Plugin for PluginOnlyCliPlugin {
    const ID: overseerd::PluginId =
        overseerd::namespaced_id!(overseerd::PluginId, "test/plugin-only-cli");

    fn cli(&self, cli: &mut PluginCliRegistrar) {
        cli.command::<PluginCatalogCommand>(
            overseerd::namespaced_id!(ContributionId, "test/plugin-catalog-command"),
            "plugin-catalog",
        );
        cli.command::<PluginBuildCommand>(
            overseerd::namespaced_id!(ContributionId, "test/plugin-build-command"),
            "plugin-build",
        );
    }

    fn contribute(self, _contributions: &mut PluginContributions) {}
}

/// Component resolved by the migration-style built command.
#[component(factory = build_marker)]
pub struct BuildMarker;

async fn build_marker() -> BuildMarker {
    COMPONENT_BUILDS.fetch_add(1, Ordering::SeqCst);

    BuildMarker
}

/// Test protocol definition accumulated by the command application.
#[derive(Default)]
pub struct TestProtocol;

impl ProtocolDefinition for TestProtocol {
    type Prepared = PreparedTestProtocol;
    type Error = overseerd_app::Error;

    const ID: overseerd::ProtocolId =
        overseerd::namespaced_id!(overseerd::ProtocolId, "test/app-commands");
    const SCOPE_TOPOLOGY: overseerd::ScopeTopology = overseerd::ScopeTopology::empty();

    fn register_plugins(plugins: &mut ProtocolPluginRegistrar) {
        plugins.mandatory(ProtocolCliPlugin::default());
    }

    fn register(&self, _registry: &mut AppRegistry) {}

    fn prepare(
        self,
        _context: &overseerd::ValidationContext<'_>,
    ) -> Result<Self::Prepared, Self::Error> {
        Ok(PreparedTestProtocol)
    }
}

/// Prepared protocol used to observe the construction boundary.
pub struct PreparedTestProtocol;

/// Built protocol runtime used to observe construction without serving.
pub struct TestRuntime;

impl PreparedProtocol for PreparedTestProtocol {
    type Runtime = TestRuntime;
    type Error = overseerd_app::Error;

    fn build(self, _runtime: &AppRuntime) -> Result<Self::Runtime, Self::Error> {
        PROTOCOL_BUILDS.fetch_add(1, Ordering::SeqCst);

        Ok(TestRuntime)
    }
}

impl ProtocolRuntime for TestRuntime {
    type Error = overseerd_app::Error;
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        PROTOCOL_DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

/// Runs after generated bootstrap but before application configuration.
#[derive(clap::Args)]
pub struct SetupCommand;

impl CliCommand<CommandApplication> for SetupCommand {
    type Phase = Setup;
    type Error = std::io::Error;

    async fn run(
        &self,
        context: CommandContext<CommandApplication, Self::Phase>,
    ) -> Result<(), Self::Error> {
        assert_eq!(
            context
                .bootstrap()
                .get::<OutputArgs>()
                .map(|args| args.format.as_str()),
            Some("json")
        );
        Ok(())
    }
}

/// Inspects registration and validation without constructing components.
#[derive(clap::Args)]
pub struct ConfiguredCommand;

impl CliCommand<CommandApplication> for ConfiguredCommand {
    type Phase = PreBuild;
    type Error = std::io::Error;

    async fn run(
        &self,
        context: CommandContext<CommandApplication, Self::Phase>,
    ) -> Result<(), Self::Error> {
        assert_eq!(context.prepared().name(), "command-app-test");

        Ok(())
    }
}

/// Lists users after building the application container.
#[derive(clap::Args)]
#[group(id = "user-list-source", required = true, multiple = false)]
pub struct ListUsersCommand {
    /// Maximum users to return.
    #[arg(long, group = "user-list-source")]
    limit: Option<usize>,

    /// List every available user.
    #[arg(long, group = "user-list-source")]
    all: bool,
}

impl CliCommand<CommandApplication> for ListUsersCommand {
    type Phase = Built;
    type Error = std::io::Error;

    async fn run(
        &self,
        context: CommandContext<CommandApplication, Self::Phase>,
    ) -> Result<(), Self::Error> {
        let marker = context
            .resolve::<std::sync::Arc<BuildMarker>>()
            .await
            .map_err(|error| {
                std::io::Error::other(format!("database marker resolution failed: {error}"))
            })?;

        assert_eq!(self.limit, Some(10));
        assert!(!self.all);
        assert_eq!(std::sync::Arc::strong_count(&marker), 2);

        Ok(())
    }
}

/// Returns a typed command failure for process-facing rendering.
#[derive(clap::Args)]
pub struct FailCommand;

impl CliCommand<CommandApplication> for FailCommand {
    type Phase = Setup;
    type Error = std::io::Error;

    async fn run(
        &self,
        _context: CommandContext<CommandApplication, Self::Phase>,
    ) -> Result<(), Self::Error> {
        Err(std::io::Error::other("intentional failure"))
    }
}

async fn setup(mut context: BootstrapContext) -> std::io::Result<BootstrapContext> {
    SETUP_CALLS.fetch_add(1, Ordering::SeqCst);
    context.insert(Vec::<&'static str>::new());

    Ok(context)
}

async fn configure(
    _context: &mut BootstrapContext,
    builder: AppBuilder<TestProtocol>,
) -> std::io::Result<AppBuilder<TestProtocol>> {
    CONFIGURE_CALLS.fetch_add(1, Ordering::SeqCst);

    Ok(builder)
}

async fn after_build(
    _context: &mut BootstrapContext,
    app: App<TestProtocol>,
) -> std::io::Result<App<TestProtocol>> {
    AFTER_BUILD_CALLS.fetch_add(1, Ordering::SeqCst);

    Ok(app)
}

async fn serve(_context: BootstrapContext, _app: App<TestProtocol>) -> std::io::Result<()> {
    SERVE_CALLS.fetch_add(1, Ordering::SeqCst);

    Ok(())
}

app! {
    pub app CommandApplication {
        name: "command-app-test",
        protocol: TestProtocol,
        managers: {
            config: ConfigManager::<Toml>::empty(),
        },
        plugins: [ApplicationCliPlugin],
        args: {
            output: OutputArgs,
        },
        commands: {
            /// Runs only application setup.
            setup_only: SetupCommand,
            /// Prepares and validates the application.
            configured: ConfiguredCommand,
            /// Administrative API commands.
            api: {
                /// Runs setup through a nested parser namespace.
                setup: SetupCommand,
                /// User administration.
                users: {
                    /// Lists users from the built application.
                    list: ListUsersCommand,
                },
            },
            fail: FailCommand,
        },
        setup = setup,
        configure = configure,
        after_build = after_build,
        serve = serve,
    }
}

app! {
    app PluginCollidingApplication {
        name: "plugin-colliding-command-test",
        protocol: (),
        plugins: [CollidingCliPlugin],
        commands: {
            inspect: InspectCommand,
        },
    }
}

app! {
    app PluginOnlyApplication {
        name: "plugin-only-command-test",
        protocol: (),
        plugins: [PluginOnlyCliPlugin],
    }
}

app! {
    app CollidingApplication {
        name: "colliding-command-test",
        protocol: (),
        args: {
            custom: CollidingArgs,
        },
        commands: {
            inspect: InspectCommand,
        },
    }
}

/// Command used by an application without a serve phase.
#[derive(clap::Args)]
pub struct InspectCommand;

impl CliCommand<CommandOnlyApplication> for InspectCommand {
    type Phase = Setup;
    type Error = std::io::Error;

    async fn run(
        &self,
        _context: CommandContext<CommandOnlyApplication, Self::Phase>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl CliCommand<CollidingApplication> for InspectCommand {
    type Phase = Setup;
    type Error = std::io::Error;

    async fn run(
        &self,
        _context: CommandContext<CollidingApplication, Self::Phase>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl CliCommand<PluginCollidingApplication> for InspectCommand {
    type Phase = Setup;
    type Error = std::io::Error;

    async fn run(
        &self,
        _context: CommandContext<PluginCollidingApplication, Self::Phase>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

app! {
    app CommandOnlyApplication {
        name: "command-only-test",
        protocol: (),
        commands: {
            inspect: InspectCommand,
        },
    }
}

#[test]
fn generated_help_contains_nested_docs_and_typed_arguments() {
    let mut command = CommandApplicationCli::command();
    let help = command.render_long_help().to_string();

    assert!(help.contains("setup-only"));
    assert!(help.contains("Administrative API commands"));
    assert!(help.contains("serve"));

    let error = match CommandApplicationCli::try_parse_from([
        "command-app-test",
        "api",
        "users",
        "list",
        "--help",
    ]) {
        Ok(_) => panic!("nested help unexpectedly parsed"),
        Err(error) => error,
    };
    let nested_help = error.to_string();

    assert!(nested_help.contains("Lists users from the built application"));
    assert!(nested_help.contains("--limit"));
    assert!(nested_help.contains("--all"));
}

fn reset_counters() {
    SETUP_CALLS.store(0, Ordering::SeqCst);
    CONFIGURE_CALLS.store(0, Ordering::SeqCst);
    COMPONENT_BUILDS.store(0, Ordering::SeqCst);
    PROTOCOL_BUILDS.store(0, Ordering::SeqCst);
    AFTER_BUILD_CALLS.store(0, Ordering::SeqCst);
    SERVE_CALLS.store(0, Ordering::SeqCst);
    PROTOCOL_PLUGIN_CONSTRUCTIONS.store(0, Ordering::SeqCst);
    APPLICATION_PLUGIN_CONSTRUCTIONS.store(0, Ordering::SeqCst);
    PROTOCOL_PLUGIN_CONTRIBUTIONS.store(0, Ordering::SeqCst);
    APPLICATION_PLUGIN_CONTRIBUTIONS.store(0, Ordering::SeqCst);
    PLUGIN_COMMAND_RUNS.store(0, Ordering::SeqCst);
    PROTOCOL_DROPS.store(0, Ordering::SeqCst);
}

#[tokio::test]
async fn parse_errors_do_not_run_setup() {
    reset_counters();

    CommandApplication::run_with(["command-app-test", "--output-format-placeholder"])
        .await
        .expect_err("unknown arguments fail before setup");

    assert_eq!(SETUP_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn setup_command_does_not_configure_or_build() {
    reset_counters();

    CommandApplication::run_with(["command-app-test", "setup-only", "--format", "json"])
        .await
        .expect("setup command runs");

    assert_eq!(SETUP_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(CONFIGURE_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(COMPONENT_BUILDS.load(Ordering::SeqCst), 0);
    assert_eq!(PROTOCOL_BUILDS.load(Ordering::SeqCst), 0);
    assert_eq!(AFTER_BUILD_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(SERVE_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn nested_setup_leaf_does_not_inherit_built_sibling_phase() {
    reset_counters();

    CommandApplication::run_with(["command-app-test", "api", "setup", "--format", "json"])
        .await
        .expect("nested setup command runs");

    assert_eq!(SETUP_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(CONFIGURE_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(COMPONENT_BUILDS.load(Ordering::SeqCst), 0);
    assert_eq!(PROTOCOL_BUILDS.load(Ordering::SeqCst), 0);
    assert_eq!(SERVE_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn application_plugin_command_composes_with_protocol_args() {
    reset_counters();

    CommandApplication::run_with([
        "command-app-test",
        "plugin-inspect",
        "--protocol-detail",
        "full",
    ])
    .await
    .expect("application plugin command runs");

    assert_eq!(PROTOCOL_PLUGIN_CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    assert_eq!(APPLICATION_PLUGIN_CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    assert_eq!(PROTOCOL_PLUGIN_CONTRIBUTIONS.load(Ordering::SeqCst), 0);
    assert_eq!(APPLICATION_PLUGIN_CONTRIBUTIONS.load(Ordering::SeqCst), 0);
    assert_eq!(PLUGIN_COMMAND_RUNS.load(Ordering::SeqCst), 1);
    assert_eq!(SETUP_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(CONFIGURE_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(SERVE_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn protocol_plugin_command_composes_with_application_plugins() {
    reset_counters();

    CommandApplication::run_with(["command-app-test", "protocol-status"])
        .await
        .expect("protocol plugin command runs");

    assert_eq!(PROTOCOL_PLUGIN_CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    assert_eq!(APPLICATION_PLUGIN_CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    assert_eq!(PLUGIN_COMMAND_RUNS.load(Ordering::SeqCst), 1);
    assert_eq!(SERVE_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn configured_app_command_consumes_the_preparse_catalog_once() {
    reset_counters();

    CommandApplication::run_with(["command-app-test", "configured"])
        .await
        .expect("configured command runs");

    assert_eq!(PROTOCOL_PLUGIN_CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    assert_eq!(APPLICATION_PLUGIN_CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    assert_eq!(PROTOCOL_PLUGIN_CONTRIBUTIONS.load(Ordering::SeqCst), 1);
    assert_eq!(APPLICATION_PLUGIN_CONTRIBUTIONS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn plugin_only_cli_reaches_configured_state() {
    reset_counters();

    PluginOnlyApplication::run_with(["plugin-only-command-test", "plugin-catalog"])
        .await
        .expect("plugin-only configured command runs");

    assert_eq!(PLUGIN_COMMAND_RUNS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn plugin_only_cli_resolves_built_dependencies() {
    reset_counters();

    PluginOnlyApplication::run_with(["plugin-only-command-test", "plugin-build"])
        .await
        .expect("plugin-only built command runs");

    assert_eq!(PLUGIN_COMMAND_RUNS.load(Ordering::SeqCst), 1);
    assert_eq!(COMPONENT_BUILDS.load(Ordering::SeqCst), 1);
}

#[test]
fn direct_generated_builder_retains_static_plugins() {
    let prepared = PluginOnlyApplication::builder()
        .expect("generated builder constructs")
        .prepare()
        .expect("generated builder prepares");

    assert!(
        prepared
            .plugin_plan()
            .resolution()
            .plugin(PluginOnlyCliPlugin::ID)
            .is_some()
    );
}

#[tokio::test]
async fn configured_command_does_not_build() {
    reset_counters();

    CommandApplication::run_with(["command-app-test", "configured"])
        .await
        .expect("configured command runs");

    assert_eq!(SETUP_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(CONFIGURE_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(COMPONENT_BUILDS.load(Ordering::SeqCst), 0);
    assert_eq!(PROTOCOL_BUILDS.load(Ordering::SeqCst), 0);
    assert_eq!(AFTER_BUILD_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(SERVE_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn built_command_resolves_components_without_serving() {
    reset_counters();

    CommandApplication::run_with(["command-app-test", "api", "users", "list", "--limit", "10"])
        .await
        .expect("built command runs");

    assert_eq!(SETUP_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(CONFIGURE_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(COMPONENT_BUILDS.load(Ordering::SeqCst), 1);
    assert_eq!(PROTOCOL_BUILDS.load(Ordering::SeqCst), 1);
    assert_eq!(AFTER_BUILD_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(SERVE_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn clap_argument_groups_are_enforced_on_leaf_commands() {
    reset_counters();

    let missing = CommandApplication::run_with(["command-app-test", "api", "users", "list"])
        .await
        .expect_err("required argument group is enforced");

    assert!(matches!(missing, CliError::Clap(_)));
    assert_eq!(SETUP_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn omitted_command_defaults_to_serve() {
    reset_counters();

    CommandApplication::run_with(["command-app-test"])
        .await
        .expect("default serve command runs");

    assert_eq!(COMPONENT_BUILDS.load(Ordering::SeqCst), 1);
    assert_eq!(PROTOCOL_BUILDS.load(Ordering::SeqCst), 1);
    assert_eq!(AFTER_BUILD_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(SERVE_CALLS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn typed_command_errors_include_the_command_path() {
    reset_counters();

    let error = CommandApplication::run_with(["command-app-test", "fail"])
        .await
        .expect_err("typed command failure is returned");

    assert!(matches!(error, CliError::Command(_)));
    assert_eq!(
        error.to_string(),
        "command `fail` failed: intentional failure"
    );
}

#[tokio::test]
async fn command_only_app_requires_a_subcommand_before_setup() {
    SETUP_CALLS.store(0, Ordering::SeqCst);

    let error = CommandOnlyApplication::run_with(["command-only-test"])
        .await
        .expect_err("command-only app requires a command");

    assert!(matches!(error, CliError::Clap(_)));
    assert_eq!(SETUP_CALLS.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn flattened_argument_collisions_return_typed_errors() {
    let error = CollidingApplication::run_with(["colliding-command-test", "inspect"])
        .await
        .expect_err("colliding arguments are rejected before Clap builds the parser");

    assert!(matches!(error, CliError::Definition(_)));
    assert_eq!(
        error.to_string(),
        "invalid command-line definition at `colliding-command-test`: duplicate long option `profile`"
    );
}

#[tokio::test]
async fn plugin_argument_collisions_name_both_contributors() {
    let error = PluginCollidingApplication::run_with(["plugin-colliding-command-test", "inspect"])
        .await
        .expect_err("plugin collision is rejected before parsing");
    let CliError::Definition(error) = error else {
        panic!("expected a CLI definition error");
    };

    assert_eq!(error.first(), overseerd::CliDefinitionSource::Framework);
    assert_eq!(
        error.second(),
        overseerd::CliDefinitionSource::Plugin(overseerd::ContributionProvenance::new(
            overseerd::Contributor::Plugin(CollidingCliPlugin::ID),
            overseerd::namespaced_id!(ContributionId, "test/colliding-args"),
        ))
    );
    assert!(error.to_string().contains("test/colliding-cli"));
}
