use std::future::Future;

use crate::{App, AppHost, BootstrapContext, PreparedApp, ProtocolDefinition};

/// The minimum application state required by a CLI command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandPhase {
    /// Bootstrap and custom setup have completed.
    Setup,
    /// Registration and validation have completed without component construction.
    Configured,
    /// Components and the application protocol have been constructed.
    Built,
}

/// A fully parsed CLI command dispatched by a generated application host.
pub trait CliCommand<H>: Sync
where
    H: AppHost,
    H::Protocol: Send,
{
    /// The typed failure returned by this command.
    type Error: std::error::Error + Send + Sync + 'static;

    /// The minimum application state this invocation requires.
    fn phase(&self) -> CommandPhase;

    /// Executes this parsed command against its requested application state.
    fn run(
        &self,
        context: CommandContext<H>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// Lifecycle-aware application state supplied to a parsed CLI command.
pub struct CommandContext<H: AppHost> {
    bootstrap: BootstrapContext,
    state: CommandState<H::Protocol>,
}

/// Application state carried by a command context.
enum CommandState<D: ProtocolDefinition> {
    Setup,
    Configured(PreparedApp<D>),
    Built(App<D>),
}

impl<H: AppHost> CommandContext<H> {
    /// Creates a setup-only command context.
    #[doc(hidden)]
    pub fn from_setup(bootstrap: BootstrapContext) -> Self {
        Self {
            bootstrap,
            state: CommandState::Setup,
        }
    }

    /// Creates a configured command context without constructing the application.
    #[doc(hidden)]
    pub fn from_configured(bootstrap: BootstrapContext, app: PreparedApp<H::Protocol>) -> Self {
        Self {
            bootstrap,
            state: CommandState::Configured(app),
        }
    }

    /// Creates a built command context.
    #[doc(hidden)]
    pub fn from_built(bootstrap: BootstrapContext, app: App<H::Protocol>) -> Self {
        Self {
            bootstrap,
            state: CommandState::Built(app),
        }
    }

    /// The application state prepared for this command.
    pub fn phase(&self) -> CommandPhase {
        match self.state {
            CommandState::Setup => CommandPhase::Setup,
            CommandState::Configured(_) => CommandPhase::Configured,
            CommandState::Built(_) => CommandPhase::Built,
        }
    }

    /// Global bootstrap state and typed global argument groups.
    pub fn bootstrap(&self) -> &BootstrapContext {
        &self.bootstrap
    }

    /// Mutable global bootstrap state and typed global argument groups.
    pub fn bootstrap_mut(&mut self) -> &mut BootstrapContext {
        &mut self.bootstrap
    }

    /// Borrows a required typed bootstrap value.
    ///
    /// Generated application argument groups and plugin argument groups are inserted by their
    /// concrete type before command dispatch. Setup hooks may insert additional typed values.
    ///
    /// # Errors
    ///
    /// Returns [`CommandContextError::MissingValue`] when no value of type `T` is present.
    pub fn require<T: Send + Sync + 'static>(&self) -> Result<&T, CommandContextError> {
        self.bootstrap
            .get::<T>()
            .ok_or(CommandContextError::MissingValue {
                type_name: std::any::type_name::<T>(),
            })
    }

    /// The prepared application, when the command requested the configured phase.
    pub fn prepared(&self) -> Option<&PreparedApp<H::Protocol>> {
        match &self.state {
            CommandState::Configured(app) => Some(app),
            CommandState::Setup | CommandState::Built(_) => None,
        }
    }

    /// Borrows the required prepared application from a configured command context.
    ///
    /// # Errors
    ///
    /// Returns [`CommandContextError::Phase`] when this command was dispatched with setup-only or
    /// built state instead of its declared configured state.
    pub fn require_prepared(&self) -> Result<&PreparedApp<H::Protocol>, CommandContextError> {
        self.prepared().ok_or(CommandContextError::Phase {
            expected: CommandPhase::Configured,
            actual: self.phase(),
        })
    }

    /// The built application, when the command requested the built phase.
    pub fn app(&self) -> Option<&App<H::Protocol>> {
        match &self.state {
            CommandState::Built(app) => Some(app),
            CommandState::Setup | CommandState::Configured(_) => None,
        }
    }

    /// Borrows the required built application from a built command context.
    ///
    /// # Errors
    ///
    /// Returns [`CommandContextError::Phase`] when this command was dispatched with setup-only or
    /// configured state instead of its declared built state.
    pub fn require_app(&self) -> Result<&App<H::Protocol>, CommandContextError> {
        self.app().ok_or(CommandContextError::Phase {
            expected: CommandPhase::Built,
            actual: self.phase(),
        })
    }

    /// Resolves an `Injectable` from the built command context's root DI container.
    ///
    /// The root container handle is shared into the returned `Send` future before asynchronous
    /// resolution begins. Command implementations therefore do not need to hold `&App` across an
    /// await, which would require protocol state to be `Sync`. This method does not consume the
    /// context, advance lifecycle phases, or start serving.
    ///
    /// # Type parameters
    ///
    /// - `T` is the concrete value or handle requested from DI and must implement `Injectable`.
    ///
    /// # Errors
    ///
    /// Returns `DiError` when this command did not request the built phase, the dependency is
    /// missing or ambiguous, a root/scope is unavailable, a stored value cannot be downcast, or
    /// transient dependency resolution or construction fails.
    pub fn resolve<T>(
        &self,
    ) -> impl Future<Output = Result<T, overseerd_di::Error>> + Send + use<H, T>
    where
        T: overseerd_di::Injectable,
    {
        let container = match &self.state {
            CommandState::Built(app) => Some(std::sync::Arc::clone(app.container())),
            CommandState::Setup | CommandState::Configured(_) => None,
        };

        async move {
            let container = container.ok_or_else(|| overseerd_di::Error::MissingDependency {
                component: std::any::type_name::<H>().to_string(),
                type_name: std::any::type_name::<T>().to_string(),
            })?;

            container
                .resolve::<T>()
                .await?
                .ok_or_else(|| overseerd_di::Error::MissingDependency {
                    component: std::any::type_name::<H>().to_string(),
                    type_name: std::any::type_name::<T>().to_string(),
                })
        }
    }

    /// Consumes a built context for framework command dispatch.
    #[doc(hidden)]
    pub fn into_built(self) -> Result<(BootstrapContext, App<H::Protocol>), CommandContextError> {
        match self.state {
            CommandState::Built(app) => Ok((self.bootstrap, app)),
            CommandState::Setup => Err(CommandContextError::Phase {
                expected: CommandPhase::Built,
                actual: CommandPhase::Setup,
            }),
            CommandState::Configured(_) => Err(CommandContextError::Phase {
                expected: CommandPhase::Built,
                actual: CommandPhase::Configured,
            }),
        }
    }

    pub(crate) fn into_plugin(self) -> super::PluginCommandContext {
        let state = match self.state {
            CommandState::Setup => super::PluginCommandState::Setup,
            CommandState::Configured(app) => {
                let (name, registry, plugin_plan) = app.into_cli_parts();

                super::PluginCommandState::Configured {
                    name,
                    registry,
                    plugin_plan,
                }
            }
            CommandState::Built(app) => {
                let (name, container, plugin_plan) = app.into_cli_parts();

                super::PluginCommandState::Built {
                    name,
                    container,
                    plugin_plan,
                }
            }
        };

        super::PluginCommandContext {
            bootstrap: self.bootstrap,
            state,
        }
    }
}

/// A generated command received application state for the wrong lifecycle phase.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CommandContextError {
    /// The generated dispatcher prepared a different phase than the command required.
    #[error("command requires {expected:?} state but received {actual:?} state")]
    Phase {
        /// The phase required by the command.
        expected: CommandPhase,
        /// The phase carried by the context.
        actual: CommandPhase,
    },
    /// A command required a typed bootstrap value that was not parsed or inserted.
    #[error("command context is missing required bootstrap value '{type_name}'")]
    MissingValue {
        /// The missing concrete Rust type name.
        type_name: &'static str,
    },
}

/// A typed leaf-command failure annotated with its complete CLI path.
#[derive(Debug, thiserror::Error)]
#[error("command `{command}` failed: {source}")]
pub struct CommandError {
    command: String,
    #[source]
    source: Box<dyn std::error::Error + Send + Sync>,
}

impl CommandError {
    /// Wraps a typed command failure with the command path users invoked.
    pub fn new(
        command: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            command: command.into(),
            source: Box::new(source),
        }
    }

    pub(crate) fn boxed(
        command: impl Into<String>,
        source: Box<dyn std::error::Error + Send + Sync>,
    ) -> Self {
        Self {
            command: command.into(),
            source,
        }
    }

    /// The complete space-separated CLI command path.
    pub fn command(&self) -> &str {
        &self.command
    }
}

#[cfg(test)]
mod tests;
