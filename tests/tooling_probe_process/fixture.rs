use upwell::{
    AppRegistry, BootstrapContext, PreparedProtocol, ProtocolDefinition, ProtocolRuntime, app,
};

const SECRET: &str = "setup panic contains api-token=probe-process-secret";

/// Protocol used by the dedicated probe-process secrecy fixture.
#[derive(Default)]
struct ProbeProtocol;

impl ProtocolDefinition for ProbeProtocol {
    type Prepared = PreparedProbeProtocol;
    type Error = upwell_app::Error;

    const ID: upwell::ProtocolId = upwell::namespaced_id!(upwell::ProtocolId, "test/probe-process");
    const SCOPE_TOPOLOGY: upwell::ScopeTopology = upwell::ScopeTopology::empty();

    fn register(&self, _registry: &mut AppRegistry) {}

    fn prepare(
        self,
        _context: &upwell::ValidationContext<'_>,
    ) -> Result<Self::Prepared, Self::Error> {
        Ok(PreparedProbeProtocol)
    }
}

/// Prepared protocol used by the dedicated probe-process secrecy fixture.
struct PreparedProbeProtocol;

/// Runtime protocol that the tooling process must never construct.
struct ProbeRuntime;

impl PreparedProtocol for PreparedProbeProtocol {
    type Runtime = ProbeRuntime;
    type Error = upwell_app::Error;

    fn build(self, _runtime: &upwell::AppRuntime) -> Result<Self::Runtime, Self::Error> {
        panic!("tooling process must not build protocol runtime");
    }

    fn tooling(&self, contributions: &mut upwell_app::ToolingContributions) {
        contributions.display(upwell_app::ResourceDisplay {
            label: Some(String::from("Probe process protocol")),
            ..Default::default()
        });
    }
}

impl ProtocolRuntime for ProbeRuntime {
    type Error = upwell_app::Error;
}

async fn panic_during_setup(_context: BootstrapContext) -> std::io::Result<BootstrapContext> {
    println!("application stdout remains independent");

    panic!("{SECRET}");
}

app! {
    app ProbeProcessApplication {
        name: "probe-process-fixture",
        protocol: ProbeProtocol,
        setup = panic_during_setup,
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = ProbeProcessApplication::run().await {
        eprintln!("upwell tooling probe process failed: {error}");
        std::process::exit(2);
    }
}
