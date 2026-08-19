use crate::DaemonApplication;
use crate::commands::OutputArgs;
use crate::lifecycle::StartupProvenance;
use upwell::{CliCommand, CommandContext, CommandContextError, PreBuild};

/// Prints the validated Homeledger plan without constructing components or the RPC protocol.
#[derive(clap::Args)]
pub struct InspectRegistryCommand;

impl CliCommand<DaemonApplication> for InspectRegistryCommand {
    type Phase = PreBuild;
    type Error = CommandContextError;

    async fn run(
        &self,
        context: CommandContext<DaemonApplication, Self::Phase>,
    ) -> Result<(), Self::Error> {
        let prepared = context.prepared();
        let verbose = context.require::<OutputArgs>()?.verbose;
        let plugins = prepared.plugin_plan().resolution().plugins();

        println!("Application: {}", prepared.name());
        println!("Protocol: {}", prepared.protocol_id());
        println!("Static plugins: {}", plugins.len());

        if let Some(provenance) = context.bootstrap().get::<StartupProvenance>() {
            println!("Config: {}", provenance.config_path);
            println!("Environments: {}", provenance.profiles.join(","));
            println!("Operator: {}", provenance.operator);
            println!("Audit review: {}", provenance.audit_review_ticket);
        }

        println!("{}", prepared.registry());

        if verbose {
            println!("Plugin plan: {:#?}", prepared.plugin_plan().resolution());
        }

        Ok(())
    }
}
