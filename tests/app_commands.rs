use std::sync::atomic::{AtomicUsize, Ordering};

use clap::{CommandFactory as _, Parser as _};
use overseerd::config::Toml;
use overseerd::{
    App, AppBuilder, AppRegistry, AppRuntime, BootstrapContext, CliCommand, CliError,
    CommandContext, CommandPhase, ConfigManager, Plugin, Protocol, ProtocolPlugin, app, component,
};

static SETUP_CALLS: AtomicUsize = AtomicUsize::new(0);
static CONFIGURE_CALLS: AtomicUsize = AtomicUsize::new(0);
static COMPONENT_BUILDS: AtomicUsize = AtomicUsize::new(0);
static PROTOCOL_BUILDS: AtomicUsize = AtomicUsize::new(0);
static AFTER_BUILD_CALLS: AtomicUsize = AtomicUsize::new(0);
static SERVE_CALLS: AtomicUsize = AtomicUsize::new(0);

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

/// Component resolved by the migration-style built command.
#[component(factory = build_marker)]
pub struct BuildMarker;

async fn build_marker() -> BuildMarker {
    COMPONENT_BUILDS.fetch_add(1, Ordering::SeqCst);

    BuildMarker
}

/// Test protocol plugin accumulated by the command application.
#[derive(Default)]
pub struct TestPlugin;

impl Plugin for TestPlugin {
    fn register(&self, _registry: &mut AppRegistry) {}
}

/// Built protocol used to observe construction without serving.
pub struct TestProtocol;

impl Protocol for TestProtocol {
    type Error = overseerd_app::Error;
}

impl ProtocolPlugin for TestPlugin {
    type Protocol = TestProtocol;
    type Error = overseerd_app::Error;

    const SCOPES: &'static [&'static dyn overseerd::Scope] = &[];

    fn build(self, _runtime: &AppRuntime) -> Result<Self::Protocol, Self::Error> {
        PROTOCOL_BUILDS.fetch_add(1, Ordering::SeqCst);

        Ok(TestProtocol)
    }
}

/// Runs after generated bootstrap but before application configuration.
#[derive(clap::Args)]
pub struct SetupCommand;

impl CliCommand<CommandApplication> for SetupCommand {
    type Error = std::io::Error;

    fn phase(&self) -> CommandPhase {
        CommandPhase::Setup
    }

    async fn run(&self, context: CommandContext<CommandApplication>) -> Result<(), Self::Error> {
        assert_eq!(context.phase(), CommandPhase::Setup);
        assert_eq!(
            context
                .bootstrap()
                .get::<OutputArgs>()
                .map(|args| args.format.as_str()),
            Some("json")
        );
        assert!(context.prepared().is_none());
        assert!(context.app().is_none());

        Ok(())
    }
}

/// Inspects registration and validation without constructing components.
#[derive(clap::Args)]
pub struct ConfiguredCommand;

impl CliCommand<CommandApplication> for ConfiguredCommand {
    type Error = std::io::Error;

    fn phase(&self) -> CommandPhase {
        CommandPhase::Configured
    }

    async fn run(&self, context: CommandContext<CommandApplication>) -> Result<(), Self::Error> {
        assert_eq!(context.phase(), CommandPhase::Configured);
        assert!(context.prepared().is_some());
        assert!(context.app().is_none());

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
    type Error = std::io::Error;

    fn phase(&self) -> CommandPhase {
        CommandPhase::Built
    }

    async fn run(&self, context: CommandContext<CommandApplication>) -> Result<(), Self::Error> {
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
    type Error = std::io::Error;

    fn phase(&self) -> CommandPhase {
        CommandPhase::Setup
    }

    async fn run(&self, _context: CommandContext<CommandApplication>) -> Result<(), Self::Error> {
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
    builder: AppBuilder<TestPlugin>,
) -> std::io::Result<AppBuilder<TestPlugin>> {
    CONFIGURE_CALLS.fetch_add(1, Ordering::SeqCst);

    Ok(builder)
}

async fn after_build(
    _context: &mut BootstrapContext,
    app: App<TestPlugin>,
) -> std::io::Result<App<TestPlugin>> {
    AFTER_BUILD_CALLS.fetch_add(1, Ordering::SeqCst);

    Ok(app)
}

async fn serve(_context: BootstrapContext, _app: App<TestPlugin>) -> std::io::Result<()> {
    SERVE_CALLS.fetch_add(1, Ordering::SeqCst);

    Ok(())
}

app! {
    pub app CommandApplication {
        name: "command-app-test",
        protocol: TestPlugin,
        managers: {
            config: ConfigManager::<Toml>::empty(),
        },
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
    type Error = std::io::Error;

    fn phase(&self) -> CommandPhase {
        CommandPhase::Setup
    }

    async fn run(
        &self,
        _context: CommandContext<CommandOnlyApplication>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl CliCommand<CollidingApplication> for InspectCommand {
    type Error = std::io::Error;

    fn phase(&self) -> CommandPhase {
        CommandPhase::Setup
    }

    async fn run(&self, _context: CommandContext<CollidingApplication>) -> Result<(), Self::Error> {
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
