use std::path::Path;

use super::TemplateKind;

const APPLICATION_CARGO: &str = r#"[package]
name = "{{ project-name }}"
version = "0.1.0"
edition = "2024"

[features]
default = ["cli"]
cli = ["upwell/cli", "dep:clap"]

[dependencies]
upwell = {{ upwell_dependency }}
clap = { version = "4", optional = true, features = ["derive"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
"#;

const APPLICATION_LIB: &str = r#"#[upwell::component]
pub struct ExampleComponent;

#[derive(Default)]
pub struct ExamplePlugin;

impl upwell::Plugin for ExamplePlugin {
    const ID: upwell::PluginId =
        upwell::namespaced_id!(upwell::PluginId, "{{ crate_name }}/example");

    fn contribute(self, _contributions: &mut upwell::PluginContributions) {}
}

#[cfg(feature = "cli")]
#[derive(clap::Args)]
pub struct AboutCommand;

#[cfg(feature = "cli")]
impl upwell::CliCommand<Application> for AboutCommand {
    type Phase = upwell::Setup;
    type Error = std::convert::Infallible;

    async fn run(
        &self,
        _context: upwell::CommandContext<Application, Self::Phase>,
    ) -> Result<(), Self::Error> {
        println!("{{ project-name }}");
        Ok(())
    }
}

upwell::app! {
    pub app Application {
        name: "{{ project-name }}",
        protocol: (),
        plugins: [ExamplePlugin],
        cli: {
            serve: false,
        },
        commands: {
            about: AboutCommand,
        },
    }
}

#[cfg(test)]
mod tests;
"#;

const APPLICATION_MAIN: &str = r#"#[cfg(feature = "cli")]
#[tokio::main]
async fn main() -> Result<(), upwell::CliError> {
    {{ crate_name }}::Application::run().await
}

#[cfg(not(feature = "cli"))]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use upwell::tooling::TOOLING_PROBE_ARGUMENT;

    let arguments = std::env::args_os().collect::<Vec<_>>();
    if arguments.len() != 2 || arguments[1] != std::ffi::OsStr::new(TOOLING_PROBE_ARGUMENT) {
        return Err("this build only supports the private Upwell tooling probe".into());
    }

    upwell::__private::install_process_probe_panic_hook();
    let target = upwell::__private::probe_target_identity_from_env()?;
    let envelope = {{ crate_name }}::Application::tooling_probe(target).await?;
    let success = envelope.is_success();

    upwell::__private::emit_probe_envelope_from_env(&envelope)?;

    if !success {
        std::process::exit(1);
    }

    Ok(())
}
"#;

const APPLICATION_TEST: &str = r#"#[test]
fn application_prepares_without_constructing_runtime_state() {
    let builder = <crate::Application as upwell::AppHost>::builder()
        .expect("application builder is available");
    let prepared = builder.prepare().expect("application prepares");

    assert_eq!(prepared.name(), "{{ project-name }}");
}
"#;

const WORKSPACE_CARGO: &str = r#"[workspace]
members = ["app"]
resolver = "3"
"#;

const PLUGIN_CARGO: &str = r#"[package]
name = "{{ project-name }}"
version = "0.1.0"
edition = "2024"

[dependencies]
upwell = {{ upwell_dependency }}
"#;

const PLUGIN_LIB: &str = r#"use std::sync::Arc;

pub struct ExampleComponent;

impl upwell::Component for ExampleComponent {
    type Handle = Arc<Self>;

    const ID: &'static str = "{{ crate_name }}_component";
    const NAME: &'static str = "ExampleComponent";

    fn into_handle(self) -> Self::Handle {
        Arc::new(self)
    }
}

impl upwell::Descriptor<upwell::ComponentDescriptor> for ExampleComponent {
    const DESCRIPTOR: upwell::ComponentDescriptor = upwell::ComponentDescriptor::of::<Self>();
}

#[derive(Default)]
pub struct ExamplePlugin;

impl upwell::Plugin for ExamplePlugin {
    const ID: upwell::PluginId =
        upwell::namespaced_id!(upwell::PluginId, "{{ crate_name }}/plugin");

    fn contribute(self, contributions: &mut upwell::PluginContributions) {
        contributions.component::<ExampleComponent>(upwell::namespaced_id!(
            upwell::ContributionId,
            "{{ crate_name }}/component"
        ));
    }
}

#[cfg(test)]
mod tests;
"#;

const PLUGIN_TEST: &str = r#"use upwell::Plugin as _;

#[test]
fn plugin_has_a_stable_identity() {
    assert_eq!(crate::ExamplePlugin::ID.as_str(), "{{ crate_name }}/plugin");
}
"#;

const PROTOCOL_CARGO: &str = r#"[package]
name = "{{ project-name }}"
version = "0.1.0"
edition = "2024"

[dependencies]
upwell-app = {{ upwell_app_dependency }}
"#;

const PROTOCOL_LIB: &str = r#"#[derive(Default)]
pub struct ExampleProtocol;

pub struct PreparedExampleProtocol;
pub struct ExampleRuntime;

impl upwell_app::ProtocolDefinition for ExampleProtocol {
    type Prepared = PreparedExampleProtocol;
    type Error = upwell_app::Error;

    const ID: upwell_app::ProtocolId =
        upwell_app::namespaced_id!(upwell_app::ProtocolId, "{{ crate_name }}/protocol");
    const SCOPE_TOPOLOGY: upwell_app::ScopeTopology = upwell_app::ScopeTopology::empty();

    fn register(&self, _registry: &mut upwell_app::AppRegistry) {}

    fn prepare(
        self,
        _context: &upwell_app::ValidationContext<'_>,
    ) -> Result<Self::Prepared, Self::Error> {
        Ok(PreparedExampleProtocol)
    }
}

impl upwell_app::PreparedProtocol for PreparedExampleProtocol {
    type Runtime = ExampleRuntime;
    type Error = upwell_app::Error;

    fn build(
        self,
        _runtime: &upwell_app::AppRuntime,
    ) -> Result<Self::Runtime, Self::Error> {
        Ok(ExampleRuntime)
    }
}

impl upwell_app::ProtocolRuntime for ExampleRuntime {
    type Error = upwell_app::Error;
}

#[cfg(test)]
mod tests;
"#;

const PROTOCOL_TEST: &str = r#"use upwell_app::ProtocolDefinition as _;

#[test]
fn protocol_has_a_stable_identity() {
    assert_eq!(
        crate::ExampleProtocol::ID.as_str(),
        "{{ crate_name }}/protocol"
    );
}
"#;

pub(super) fn materialize_builtin(kind: TemplateKind, root: &Path) -> std::io::Result<()> {
    std::fs::write(root.join("cargo-generate.toml"), generator_config())?;

    match kind {
        TemplateKind::Application => application(root),
        TemplateKind::ApplicationWorkspace => application_workspace(root),
        TemplateKind::Plugin => plugin(root),
        TemplateKind::Protocol => protocol(root),
    }
}

fn generator_config() -> &'static str {
    r#"[template]
cargo_generate_version = ">=0.23.14"

[placeholders.upwell_version]
type = "string"
prompt = "Upwell version"

[placeholders.upwell_dependency]
type = "string"
prompt = "Upwell dependency"

[placeholders.upwell_app_dependency]
type = "string"
prompt = "Upwell app dependency"
"#
}

fn application(root: &Path) -> std::io::Result<()> {
    write(root, "Cargo.toml", APPLICATION_CARGO)?;
    write(root, "src/lib.rs", APPLICATION_LIB)?;
    write(root, "src/main.rs", APPLICATION_MAIN)?;
    write(root, "src/tests.rs", APPLICATION_TEST)
}

fn application_workspace(root: &Path) -> std::io::Result<()> {
    write(root, "Cargo.toml", WORKSPACE_CARGO)?;
    write(root, "app/Cargo.toml", APPLICATION_CARGO)?;
    write(root, "app/src/lib.rs", APPLICATION_LIB)?;
    write(root, "app/src/main.rs", APPLICATION_MAIN)?;
    write(root, "app/src/tests.rs", APPLICATION_TEST)
}

fn plugin(root: &Path) -> std::io::Result<()> {
    write(root, "Cargo.toml", PLUGIN_CARGO)?;
    write(root, "src/lib.rs", PLUGIN_LIB)?;
    write(root, "src/tests.rs", PLUGIN_TEST)
}

fn protocol(root: &Path) -> std::io::Result<()> {
    write(root, "Cargo.toml", PROTOCOL_CARGO)?;
    write(root, "src/lib.rs", PROTOCOL_LIB)?;
    write(root, "src/tests.rs", PROTOCOL_TEST)
}

fn write(root: &Path, relative: &str, contents: &str) -> std::io::Result<()> {
    let path = root.join(relative);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(path, contents)
}
