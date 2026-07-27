use crate::DaemonApplication;
use crate::commands::OutputArgs;
use overseerd::{CliCommand, CommandContext, CommandContextError, CommandPhase};

/// Prints the validated registry without constructing components or the RPC protocol.
#[derive(clap::Args)]
pub struct InspectRegistryCommand;

impl CliCommand<DaemonApplication> for InspectRegistryCommand {
    type Error = CommandContextError;

    fn phase(&self) -> CommandPhase {
        CommandPhase::Configured
    }

    async fn run(&self, context: CommandContext<DaemonApplication>) -> Result<(), Self::Error> {
        let prepared = context.require_prepared()?;
        let verbose = context.require::<OutputArgs>()?.verbose;

        println!("Application: {}", prepared.name());
        println!("{}", prepared.registry());

        if verbose {
            println!(
                "Protocol: {}",
                std::any::type_name_of_val(prepared.protocol())
            );
        }

        Ok(())
    }
}
