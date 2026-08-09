#![cfg(feature = "cli")]

use std::sync::atomic::{AtomicUsize, Ordering};

use clap::{CommandFactory as _, Parser as _};
use upwell::config::Toml;
use upwell::{
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

#[cfg(feature = "tooling")]
static CUSTOMIZED_TOOLING_BOOTSTRAP: std::sync::Mutex<Option<CustomizedToolingBootstrap>> =
    std::sync::Mutex::new(None);

/// Generated tooling bootstrap values captured by customized setup.
#[cfg(feature = "tooling")]
struct CustomizedToolingBootstrap {
    profiles: Vec<String>,
    log: String,
    format: upwell::LogFormat,
    color: upwell::ColorChoice,
}

#[cfg(feature = "tooling")]
fn tooling_target(binary: &str) -> upwell::tooling::ProbeTargetIdentity {
    upwell::tooling::ProbeTargetIdentity::new(
        upwell::tooling::PackageIdentity {
            name: String::from("upwell"),
            version: Some(String::from(env!("CARGO_PKG_VERSION"))),
            manifest_path: Some(format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR"))),
        },
        upwell::tooling::BinaryTargetIdentity {
            name: binary.to_string(),
        },
    )
    .expect("test target identity is valid")
}

/// Global arguments flattened into the generated application parser.
#[derive(clap::Args)]
pub struct OutputArgs {
    /// Output representation used by utility commands.
    #[arg(long, global = true, default_value = "text")]
    format: String,
}

/// Application arguments completing all generated default declaration forms.
#[derive(clap::Args)]
pub struct DefaultFormsArgs {
    /// Default output channels used by customized inspection.
    #[arg(long, global = true, default_values = ["audit", "metrics"])]
    channels: Vec<String>,
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
    const ID: upwell::PluginId = upwell::namespaced_id!(upwell::PluginId, "test/colliding-cli");

    fn contribute(self, _contributions: &mut PluginContributions) {}

    fn cli(&self, cli: &mut PluginCliRegistrar) {
        cli.args::<CollidingPluginArgs>(upwell::namespaced_id!(
            ContributionId,
            "test/colliding-args"
        ));
    }
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
    const ID: upwell::PluginId = upwell::namespaced_id!(upwell::PluginId, "test/protocol-cli");

    fn contribute(self, _contributions: &mut PluginContributions) {
        assert!(self.marker);

        PROTOCOL_PLUGIN_CONTRIBUTIONS.fetch_add(1, Ordering::SeqCst);
    }

    fn cli(&self, cli: &mut PluginCliRegistrar) {
        cli.args::<ProtocolPluginArgs>(upwell::namespaced_id!(
            ContributionId,
            "test/protocol-args"
        ));
        cli.commands::<ProtocolPluginCommands>(upwell::namespaced_id!(
            ContributionId,
            "test/protocol-commands"
        ));
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
    const ID: upwell::PluginId = upwell::namespaced_id!(upwell::PluginId, "test/application-cli");

    fn contribute(self, _contributions: &mut PluginContributions) {
        APPLICATION_PLUGIN_CONTRIBUTIONS.fetch_add(1, Ordering::SeqCst);
    }

    fn cli(&self, cli: &mut PluginCliRegistrar) {
        cli.command::<PluginInspectCommand>(
            upwell::namespaced_id!(ContributionId, "test/plugin-inspect-command"),
            "plugin-inspect",
        );
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
    type Error = upwell::DiError;

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
    const ID: upwell::PluginId = upwell::namespaced_id!(upwell::PluginId, "test/plugin-only-cli");

    fn contribute(self, _contributions: &mut PluginContributions) {}

    fn cli(&self, cli: &mut PluginCliRegistrar) {
        cli.command::<PluginCatalogCommand>(
            upwell::namespaced_id!(ContributionId, "test/plugin-catalog-command"),
            "plugin-catalog",
        );
        cli.command::<PluginBuildCommand>(
            upwell::namespaced_id!(ContributionId, "test/plugin-build-command"),
            "plugin-build",
        );
    }
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
    type Error = upwell_app::Error;

    const ID: upwell::ProtocolId = upwell::namespaced_id!(upwell::ProtocolId, "test/app-commands");
    const SCOPE_TOPOLOGY: upwell::ScopeTopology = upwell::ScopeTopology::empty();

    fn register(&self, _registry: &mut AppRegistry) {}

    fn prepare(
        self,
        _context: &upwell::ValidationContext<'_>,
    ) -> Result<Self::Prepared, Self::Error> {
        Ok(PreparedTestProtocol)
    }

    fn register_plugins(plugins: &mut ProtocolPluginRegistrar) {
        plugins.mandatory(ProtocolCliPlugin::default());
    }
}

/// Prepared protocol used to observe the construction boundary.
pub struct PreparedTestProtocol;

/// Built protocol runtime used to observe construction without serving.
pub struct TestRuntime;

impl PreparedProtocol for PreparedTestProtocol {
    type Runtime = TestRuntime;
    type Error = upwell_app::Error;

    fn build(self, _runtime: &AppRuntime) -> Result<Self::Runtime, Self::Error> {
        PROTOCOL_BUILDS.fetch_add(1, Ordering::SeqCst);

        Ok(TestRuntime)
    }

    #[cfg(feature = "tooling")]
    fn tooling(&self, contributions: &mut upwell_app::ToolingContributions) {
        contributions.display(upwell_app::ResourceDisplay {
            label: Some(String::from("Command test protocol")),
            ..Default::default()
        });
    }
}

impl ProtocolRuntime for TestRuntime {
    type Error = upwell_app::Error;
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

    #[cfg(feature = "tooling")]
    if context.mode().is_tooling() {
        assert_eq!(
            context
                .get::<OutputArgs>()
                .expect("tooling parses application globals")
                .format,
            "text"
        );
        assert_eq!(
            context
                .get::<ProtocolPluginArgs>()
                .expect("tooling parses plugin globals")
                .protocol_detail,
            "summary"
        );
    }

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

async fn serve_unit(_context: BootstrapContext, _app: App<()>) -> std::io::Result<()> {
    SERVE_CALLS.fetch_add(1, Ordering::SeqCst);

    Ok(())
}

async fn customized_setup(context: BootstrapContext) -> std::io::Result<BootstrapContext> {
    #[cfg(feature = "tooling")]
    if context.mode().is_tooling() {
        let bootstrap = context
            .bootstrap()
            .expect("customized tooling bootstrap state exists");
        let snapshot = CustomizedToolingBootstrap {
            profiles: bootstrap.profiles().to_vec(),
            log: bootstrap.logging().level.clone(),
            format: bootstrap.logging().format,
            color: bootstrap.color(),
        };

        *CUSTOMIZED_TOOLING_BOOTSTRAP
            .lock()
            .expect("customized tooling bootstrap lock is available") = Some(snapshot);
    }

    Ok(context)
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

impl CliCommand<CustomizedCollisionApplication> for InspectCommand {
    type Phase = Setup;
    type Error = std::io::Error;

    async fn run(
        &self,
        _context: CommandContext<CustomizedCollisionApplication, Self::Phase>,
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

app! {
    app CustomizedCliApplication {
        name: "customized-cli-test",
        protocol: (),
        cli: {
            config: false,
            profile: {
                name: "environment",
                short: 'e',
                aliases: ["profile"],
                help: "Configuration environment.",
                value_name: "ENV",
                default_values_t: [String::from("local")],
            },
            log: { default_value: "warn" },
            log_format: { default_value_t: upwell::LogFormat::Json },
            color: { default_value: "never" },
            serve: {
                name: "start",
                visible_aliases: ["run"],
                help: "Start the customized application.",
                default_command: false,
            },
        },
        args: {
            defaults: DefaultFormsArgs,
        },
        commands: {
            inspect: CustomizedInspectCommand,
        },
        setup = customized_setup,
        serve = serve_unit,
    }
}

app! {
    app DisabledServeApplication {
        name: "disabled-serve-test",
        protocol: (),
        cli: { serve: false },
        commands: {
            serve: DisabledServeCommand,
        },
        serve = serve_unit,
    }
}

app! {
    app CustomizedCollisionApplication {
        name: "customized-collision-test",
        protocol: (),
        cli: {
            profile: { name: "environment", short: false },
        },
        plugins: [CollidingCliPlugin],
        commands: {
            inspect: InspectCommand,
        },
    }
}

/// Setup command observing customized bootstrap semantics.
#[derive(clap::Args)]
pub struct CustomizedInspectCommand {
    /// Assert explicit CLI values rather than generated parser defaults.
    #[arg(long)]
    expect_cli: bool,
}

impl CliCommand<CustomizedCliApplication> for CustomizedInspectCommand {
    type Phase = Setup;
    type Error = std::io::Error;

    async fn run(
        &self,
        context: CommandContext<CustomizedCliApplication, Self::Phase>,
    ) -> Result<(), Self::Error> {
        let bootstrap = context
            .bootstrap()
            .bootstrap()
            .expect("generated bootstrap state exists");

        if self.expect_cli {
            assert_eq!(bootstrap.profiles(), ["production", "regional"]);
            assert_eq!(bootstrap.logging().level, "trace,customized=debug");
            assert_eq!(bootstrap.logging().format, upwell::LogFormat::Pretty);
            assert_eq!(bootstrap.color(), upwell::ColorChoice::Always);
        } else {
            assert_eq!(bootstrap.profiles(), ["local"]);
            assert_eq!(bootstrap.logging().level, "warn");
            assert_eq!(bootstrap.logging().format, upwell::LogFormat::Json);
            assert_eq!(bootstrap.color(), upwell::ColorChoice::Never);
        }

        Ok(())
    }
}

/// Application-owned serve command enabled after framework serve opt-out.
#[derive(clap::Args)]
pub struct DisabledServeCommand;

impl CliCommand<DisabledServeApplication> for DisabledServeCommand {
    type Phase = Setup;
    type Error = std::io::Error;

    async fn run(
        &self,
        _context: CommandContext<DisabledServeApplication, Self::Phase>,
    ) -> Result<(), Self::Error> {
        Ok(())
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

#[cfg(feature = "tooling")]
#[tokio::test]
async fn tooling_probe_projects_effective_application_and_plugin_cli_metadata() {
    let envelope = CommandApplication::tooling_probe(tooling_target("command-bin"))
        .await
        .expect("generated declaration identity validates");
    let upwell::tooling::ProbeOutcome::Success { document } = envelope.outcome else {
        panic!("command application tooling probe failed");
    };
    let cli = document
        .cli
        .expect("CLI-enabled probe emits typed metadata");
    let output = cli
        .root
        .arguments
        .iter()
        .find(|argument| argument.id == "format")
        .expect("application args metadata exists");
    let profile = cli
        .root
        .arguments
        .iter()
        .find(|argument| argument.id == "profiles")
        .expect("framework profile metadata exists");
    let api = cli
        .root
        .commands
        .iter()
        .find(|command| command.name == "api")
        .expect("nested application command exists");
    let plugin_inspect = cli
        .root
        .commands
        .iter()
        .find(|command| command.name == "plugin-inspect")
        .expect("named plugin command exists");
    let protocol_status = cli
        .root
        .commands
        .iter()
        .find(|command| command.name == "protocol-status")
        .expect("plugin command-set entry exists");
    let serve = cli
        .root
        .commands
        .iter()
        .find(|command| command.name == "serve")
        .expect("framework serve command exists");
    let root_help = cli
        .root
        .arguments
        .iter()
        .find(|argument| argument.id == "help")
        .expect("generated root help exists");
    let root_version = cli
        .root
        .arguments
        .iter()
        .find(|argument| argument.id == "version")
        .expect("generated root version exists");
    let api_help = api
        .arguments
        .iter()
        .find(|argument| argument.id == "help")
        .expect("generated nested help exists");

    assert!(matches!(
        output.owner,
        upwell::tooling::CliOwner::Application
    ));
    assert!(matches!(
        profile.owner,
        upwell::tooling::CliOwner::Framework
    ));
    assert!(matches!(api.owner, upwell::tooling::CliOwner::Application));
    assert!(matches!(
        plugin_inspect.owner,
        upwell::tooling::CliOwner::Plugin { .. }
    ));
    assert!(matches!(
        protocol_status.owner,
        upwell::tooling::CliOwner::Plugin { .. }
    ));
    assert!(matches!(serve.owner, upwell::tooling::CliOwner::Framework));
    assert_eq!(serve.id.as_deref(), Some("serve"));
    assert_eq!(cli.default_command.as_deref(), Some("serve"));
    assert!(matches!(
        root_help.owner,
        upwell::tooling::CliOwner::Framework
    ));
    assert!(matches!(
        root_version.owner,
        upwell::tooling::CliOwner::Framework
    ));
    assert!(matches!(
        api_help.owner,
        upwell::tooling::CliOwner::Framework
    ));
    assert!(cli.providers.iter().any(|provider| {
        provider.contribution == "test/protocol-args"
            && provider.kind == upwell::tooling::CliProviderKind::Args
    }));
    assert!(cli.providers.iter().any(|provider| {
        provider.contribution == "test/plugin-inspect-command"
            && provider.kind == upwell::tooling::CliProviderKind::Command
    }));
    assert!(cli.providers.iter().any(|provider| {
        provider.contribution == "test/protocol-commands"
            && provider.kind == upwell::tooling::CliProviderKind::CommandSet
    }));
    assert_eq!(
        api.commands
            .iter()
            .find(|command| command.name == "users")
            .and_then(|command| {
                command
                    .commands
                    .iter()
                    .find(|command| command.name == "list")
            })
            .map(|command| command.name.as_str()),
        Some("list")
    );
}

#[cfg(feature = "tooling")]
#[tokio::test]
async fn tooling_projects_customized_framework_shape_from_the_effective_parser() {
    let envelope = CustomizedCliApplication::tooling_probe(tooling_target("customized-bin"))
        .await
        .expect("generated declaration identity validates");
    let upwell::tooling::ProbeOutcome::Success { document } = envelope.outcome else {
        panic!("customized CLI tooling probe failed");
    };
    let cli = document.cli.expect("CLI metadata exists");
    let profile = cli
        .root
        .arguments
        .iter()
        .find(|argument| argument.id == "profiles")
        .expect("customized profile exists");
    let serve = cli
        .root
        .commands
        .iter()
        .find(|command| command.name == "start")
        .expect("customized serve exists");
    let channels = cli
        .root
        .arguments
        .iter()
        .find(|argument| argument.id == "channels")
        .expect("application default-values argument exists");
    let log = cli
        .root
        .arguments
        .iter()
        .find(|argument| argument.id == "log")
        .expect("literal scalar default exists");
    let log_format = cli
        .root
        .arguments
        .iter()
        .find(|argument| argument.id == "log_format")
        .expect("typed scalar default exists");
    let color = cli
        .root
        .arguments
        .iter()
        .find(|argument| argument.id == "color")
        .expect("literal choice default exists");

    assert_eq!(profile.long.as_deref(), Some("environment"));
    assert_eq!(profile.short, Some('e'));
    assert_eq!(profile.aliases, ["profile"]);
    assert!(matches!(
        profile.owner,
        upwell::tooling::CliOwner::Framework
    ));
    assert_eq!(profile.default_values, ["local"]);
    assert_eq!(profile.id, "profiles");
    assert_eq!(channels.default_values, ["audit", "metrics"]);
    assert_eq!(log.default_values, ["warn"]);
    assert_eq!(log_format.default_values, ["json"]);
    assert_eq!(color.default_values, ["never"]);
    assert_eq!(serve.visible_aliases, ["run"]);
    assert_eq!(serve.id.as_deref(), Some("serve"));
    assert_eq!(cli.default_command, None);
    assert!(matches!(serve.owner, upwell::tooling::CliOwner::Framework));
    let snapshot = CUSTOMIZED_TOOLING_BOOTSTRAP
        .lock()
        .expect("customized tooling bootstrap lock is available")
        .take()
        .expect("customized tooling setup captured bootstrap state");

    assert_eq!(snapshot.profiles, ["local"]);
    assert_eq!(snapshot.log, "warn");
    assert_eq!(snapshot.format, upwell::LogFormat::Json);
    assert_eq!(snapshot.color, upwell::ColorChoice::Never);

    let envelope = CustomizedCliApplication::tooling_probe(tooling_target("customized-bin"))
        .await
        .expect("second generated tooling probe succeeds");
    let json = envelope.to_json().expect("probe envelope serializes");
    let decoded = upwell::tooling::ProbeEnvelope::from_json(&json)
        .expect("probe envelope metadata round-trips through process JSON");
    let upwell::tooling::ProbeOutcome::Success { document } = decoded.outcome else {
        panic!("round-tripped customized CLI tooling probe failed");
    };
    let cli = document.cli.expect("round-tripped CLI metadata exists");

    assert_eq!(
        cli.root
            .arguments
            .iter()
            .find(|argument| argument.id == "profiles")
            .and_then(|argument| argument.long.as_deref()),
        Some("environment")
    );
    assert!(
        cli.root
            .arguments
            .iter()
            .all(|argument| argument.id != "config")
    );
    assert_eq!(
        cli.root
            .commands
            .iter()
            .find(|command| command.id.as_deref() == Some("serve"))
            .map(|command| command.name.as_str()),
        Some("start")
    );
}

#[cfg(feature = "tooling")]
#[tokio::test]
async fn tooling_projects_generated_host_lifecycle_capabilities_without_invented_callbacks() {
    let envelope = CommandApplication::tooling_probe(tooling_target("command-bin"))
        .await
        .expect("generated declaration identity validates");
    let upwell::tooling::ProbeOutcome::Success { document } = envelope.outcome else {
        panic!("command application tooling probe failed");
    };

    for (phase, callback) in [
        ("setup", "true"),
        ("configure", "true"),
        ("before_build", "false"),
        ("after_build", "true"),
        ("serve", "true"),
    ] {
        let lifecycle = document
            .resources
            .iter()
            .find(|resource| resource.id == format!("lifecycle:{phase}"))
            .expect("generated lifecycle resource exists");

        assert_eq!(lifecycle.labels["callback"], callback);
    }

    for phase in ["prepare", "build", "startup", "shutdown", "config_reload"] {
        assert!(
            document
                .resources
                .iter()
                .any(|resource| resource.id == format!("lifecycle:{phase}"))
        );
    }
}

#[test]
fn customized_framework_parser_exposes_only_effective_behavior() {
    let mut command = CustomizedCliApplicationCli::command();
    let help = command.render_long_help().to_string();

    assert!(!help.contains("--config"));
    assert!(help.contains("--environment <ENV>"));
    assert!(help.contains("[default: local]"));
    assert!(help.contains("[default: warn]"));
    assert!(help.contains("[default: json]"));
    assert!(help.contains("[default: never]"));
    assert!(help.contains("--channels <CHANNELS>"));
    assert!(help.contains("audit"));
    assert!(help.contains("metrics"));
    assert!(help.contains("start"));
    assert!(help.contains("Start the customized application."));
    assert!(
        CustomizedCliApplicationCli::try_parse_from([
            "customized-cli-test",
            "--profile",
            "production",
            "run",
        ])
        .is_ok()
    );
}

#[test]
fn generated_parser_marks_application_defaults_as_default_values() {
    let mut command = CustomizedCliApplicationCli::command();
    let matches = command
        .try_get_matches_from_mut(["customized-cli-test", "inspect"])
        .expect("customized defaults parse");

    for id in ["profiles", "log", "log_format", "color", "channels"] {
        assert_eq!(
            matches.value_source(id),
            Some(clap::parser::ValueSource::DefaultValue)
        );
    }
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
async fn customized_parser_defaults_and_serve_dispatch_are_typed() {
    let serve_cli = CustomizedCliApplicationCli::try_parse_from(["customized-cli-test", "run"])
        .expect("visible serve alias parses");

    assert!(matches!(
        serve_cli.command,
        Some(CustomizedCliApplicationCommand::Serve)
    ));

    CustomizedCliApplication::run_with(["customized-cli-test", "inspect"])
        .await
        .expect("customized bootstrap defaults apply");

    let missing = CustomizedCliApplication::run_with(["customized-cli-test"])
        .await
        .expect_err("serve is not the default command");

    assert!(matches!(missing, CliError::Clap(_)));
}

#[tokio::test]
async fn explicit_cli_values_override_customized_parser_defaults() {
    CustomizedCliApplication::run_with([
        "customized-cli-test",
        "inspect",
        "--expect-cli",
        "--environment",
        "production",
        "--profile",
        "regional",
        "--log",
        "trace,customized=debug",
        "--log-format",
        "pretty",
        "--color",
        "always",
    ])
    .await
    .expect("explicit CLI values override application bootstrap defaults");
}

#[tokio::test]
async fn disabled_framework_serve_allows_application_owned_command() {
    DisabledServeApplication::run_with(["disabled-serve-test", "serve"])
        .await
        .expect("application-owned serve command runs");

    assert_eq!(SERVE_CALLS.load(Ordering::SeqCst), 0);
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
        "invalid command-line definition at `colliding-command-test`: duplicate long option `profile` from Framework and Application"
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

    assert_eq!(error.first(), upwell::CliDefinitionSource::Framework);
    assert_eq!(
        error.second(),
        upwell::CliDefinitionSource::Plugin(upwell::ContributionProvenance::new(
            upwell::Contributor::Plugin(CollidingCliPlugin::ID),
            upwell::namespaced_id!(ContributionId, "test/colliding-args"),
        ))
    );
    assert!(error.to_string().contains("test/colliding-cli"));
}

#[tokio::test]
async fn renamed_framework_slot_releases_its_default_name() {
    CustomizedCollisionApplication::run_with(["customized-collision-test", "inspect"])
        .await
        .expect("plugin can use a released framework option name");
}
