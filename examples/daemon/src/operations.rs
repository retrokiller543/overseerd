//! A statically installed operations plugin with generated CLI contributions.

use upwell::{
    ContributionId, Plugin, PluginCliCommand, PluginCliRegistrar, PluginCommandContext,
    PluginContributions, PluginId, Setup, component, namespaced_id,
};

/// Homeledger household ledger daemon and administration host.
#[derive(clap::Args)]
pub struct OperationsArgs {
    /// Operator recorded in Homeledger administration output.
    #[arg(long, global = true, default_value = "local-admin")]
    pub(crate) operator: String,
}

/// Prints generated bootstrap choices without preparing or building Homeledger.
#[derive(clap::Args)]
pub struct OperatorContextCommand;

impl PluginCliCommand for OperatorContextCommand {
    type Phase = Setup;
    type Error = upwell::CommandContextError;

    async fn run(&self, context: PluginCommandContext<Self::Phase>) -> Result<(), Self::Error> {
        let arguments = context.require::<OperationsArgs>()?;
        let bootstrap = context.bootstrap().bootstrap();

        println!("operator: {}", arguments.operator);

        if let Some(bootstrap) = bootstrap {
            println!("config: {}", bootstrap.config_path().display());
            println!("environments: {}", bootstrap.profiles().join(","));
            println!("log filter: {}", bootstrap.logging().level);
            println!("log format: {:?}", bootstrap.logging().format);
        }

        Ok(())
    }
}

/// Runtime marker contributed by the Homeledger operations plugin.
#[component]
pub struct OperationsRegistry;

/// Statically installed operations support for both CLI and runtime assembly.
#[derive(Default)]
pub struct HomeledgerOperationsPlugin;

impl Plugin for HomeledgerOperationsPlugin {
    const ID: PluginId = namespaced_id!(PluginId, "homeledger/operations");

    fn contribute(self, contributions: &mut PluginContributions) {
        contributions.component::<OperationsRegistry>(namespaced_id!(
            ContributionId,
            "homeledger/operations-registry"
        ));
    }

    fn cli(&self, cli: &mut PluginCliRegistrar) {
        cli.args::<OperationsArgs>(namespaced_id!(ContributionId, "homeledger/operator-args"));
        cli.command::<OperatorContextCommand>(
            namespaced_id!(ContributionId, "homeledger/operator-context"),
            "operator-context",
        );
    }
}
