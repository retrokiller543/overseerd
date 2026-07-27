use std::future::Future;

use crate::{
    AppRegistry, BootstrapContext, CommandContextError, CommandPhase, EffectivePluginPlan,
};

/// A typed plugin command that participates in generated lifecycle-aware dispatch.
pub trait PluginCliCommand: Sync {
    /// The typed failure returned by this command.
    type Error: std::error::Error + Send + Sync + 'static;

    /// The minimum application state this invocation requires.
    fn phase(&self) -> CommandPhase;

    /// Executes this parsed command against protocol-neutral lifecycle state.
    fn run(
        &self,
        context: PluginCommandContext,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// Protocol-neutral lifecycle state supplied to a selected plugin command.
pub struct PluginCommandContext {
    pub(crate) bootstrap: BootstrapContext,
    pub(crate) state: PluginCommandState,
}

pub(crate) enum PluginCommandState {
    Setup,
    Configured {
        name: String,
        registry: AppRegistry,
        plugin_plan: EffectivePluginPlan,
    },
    Built {
        name: String,
        container: std::sync::Arc<overseerd_di::ScopeContainer>,
        plugin_plan: EffectivePluginPlan,
        _owner: Box<dyn Send>,
    },
}

impl PluginCommandContext {
    /// The lifecycle phase prepared for this command.
    pub fn phase(&self) -> CommandPhase {
        match self.state {
            PluginCommandState::Setup => CommandPhase::Setup,
            PluginCommandState::Configured { .. } => CommandPhase::Configured,
            PluginCommandState::Built { .. } => CommandPhase::Built,
        }
    }

    /// Global bootstrap state and typed application/plugin argument groups.
    pub const fn bootstrap(&self) -> &BootstrapContext {
        &self.bootstrap
    }

    /// Mutable global bootstrap state and typed application/plugin argument groups.
    pub fn bootstrap_mut(&mut self) -> &mut BootstrapContext {
        &mut self.bootstrap
    }

    /// Borrows a required typed bootstrap value.
    pub fn require<T: Send + Sync + 'static>(&self) -> Result<&T, CommandContextError> {
        self.bootstrap
            .get::<T>()
            .ok_or(CommandContextError::MissingValue {
                type_name: std::any::type_name::<T>(),
            })
    }

    /// The configured or built application name, when available.
    pub fn application_name(&self) -> Option<&str> {
        match &self.state {
            PluginCommandState::Setup => None,
            PluginCommandState::Configured { name, .. }
            | PluginCommandState::Built { name, .. } => Some(name),
        }
    }

    /// The validated registry for a configured command.
    pub fn registry(&self) -> Option<&AppRegistry> {
        match &self.state {
            PluginCommandState::Configured { registry, .. } => Some(registry),
            PluginCommandState::Setup | PluginCommandState::Built { .. } => None,
        }
    }

    /// The immutable effective plugin plan for configured or built commands.
    pub fn plugin_plan(&self) -> Option<&EffectivePluginPlan> {
        match &self.state {
            PluginCommandState::Setup => None,
            PluginCommandState::Configured { plugin_plan, .. }
            | PluginCommandState::Built { plugin_plan, .. } => Some(plugin_plan),
        }
    }

    /// Resolves an `Injectable` from a built plugin command's root DI container.
    pub fn resolve<T>(&self) -> impl Future<Output = Result<T, overseerd_di::Error>> + Send + use<T>
    where
        T: overseerd_di::Injectable,
    {
        let container = match &self.state {
            PluginCommandState::Built { container, .. } => Some(std::sync::Arc::clone(container)),
            PluginCommandState::Setup | PluginCommandState::Configured { .. } => None,
        };

        async move {
            let container = container.ok_or_else(|| overseerd_di::Error::MissingDependency {
                component: String::from("plugin CLI command"),
                type_name: std::any::type_name::<T>().to_string(),
            })?;

            container
                .resolve::<T>()
                .await?
                .ok_or_else(|| overseerd_di::Error::MissingDependency {
                    component: String::from("plugin CLI command"),
                    type_name: std::any::type_name::<T>().to_string(),
                })
        }
    }
}
