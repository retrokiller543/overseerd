//! Reusable application-host lifecycle contracts.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;

use crate::{AppBuilder, ProtocolPlugin};

#[cfg(feature = "cli")]
mod cli;

#[cfg(feature = "cli")]
pub use cli::*;

/// Selects whether a host is executing normally or preparing metadata for developer tooling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExecutionMode {
    /// Normal application construction and execution.
    Run,
    /// Side-effect-free application preparation for developer tooling.
    Tooling,
}

impl ExecutionMode {
    /// Whether this execution may proceed into normal runtime behavior.
    pub fn is_run(self) -> bool {
        matches!(self, Self::Run)
    }

    /// Whether this execution is restricted to tooling-safe preparation.
    pub fn is_tooling(self) -> bool {
        matches!(self, Self::Tooling)
    }
}

/// Identifies the lifecycle phase in which a host operation failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LifecyclePhase {
    /// Bootstrap context creation.
    Setup,
    /// Application builder creation and configuration.
    Configure,
    /// Final builder customization before validation.
    BeforeBuild,
    /// Registration and validation without component construction.
    Prepare,
    /// Component and protocol construction.
    Build,
    /// Post-construction customization.
    AfterBuild,
    /// Long-running protocol execution.
    Serve,
}

impl fmt::Display for LifecyclePhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Setup => "setup",
            Self::Configure => "configure",
            Self::BeforeBuild => "before_build",
            Self::Prepare => "prepare",
            Self::Build => "build",
            Self::AfterBuild => "after_build",
            Self::Serve => "serve",
        };

        formatter.write_str(name)
    }
}

/// Typed values and execution metadata shared across generated host lifecycle phases.
pub struct BootstrapContext {
    mode: ExecutionMode,
    extensions: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl BootstrapContext {
    /// Creates an empty context for `mode`.
    pub fn new(mode: ExecutionMode) -> Self {
        Self {
            mode,
            extensions: HashMap::new(),
        }
    }

    /// The execution mode selected by the host runner.
    pub fn mode(&self) -> ExecutionMode {
        self.mode
    }

    /// Inserts a typed lifecycle value, returning the previous value of that type if present.
    pub fn insert<T: Send + Sync + 'static>(&mut self, value: T) -> Option<T> {
        self.extensions
            .insert(TypeId::of::<T>(), Box::new(value))
            .and_then(|previous| previous.downcast::<T>().ok())
            .map(|previous| *previous)
    }

    /// Borrows a typed lifecycle value.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.extensions.get(&TypeId::of::<T>())?.downcast_ref()
    }

    /// Mutably borrows a typed lifecycle value.
    pub fn get_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        self.extensions.get_mut(&TypeId::of::<T>())?.downcast_mut()
    }

    /// Removes and returns a typed lifecycle value.
    pub fn remove<T: Send + Sync + 'static>(&mut self) -> Option<T> {
        self.extensions
            .remove(&TypeId::of::<T>())?
            .downcast::<T>()
            .ok()
            .map(|value| *value)
    }
}

/// A lifecycle failure tagged with the phase that produced it.
#[derive(Debug, thiserror::Error)]
#[error("{phase} phase failed: {source}")]
pub struct PhaseError {
    phase: LifecyclePhase,
    #[source]
    source: Box<dyn std::error::Error + Send + Sync>,
}

/// Host-runner failures that occur outside user lifecycle callbacks.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HostError {
    /// Tooling execution attempted to construct ordinary components or a served protocol.
    #[error("tooling mode cannot construct application components or protocols")]
    ToolingConstruction,
}

impl PhaseError {
    /// Wraps a typed lifecycle error with its phase.
    pub fn new(
        phase: LifecyclePhase,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            phase,
            source: Box::new(source),
        }
    }

    /// The phase that failed.
    pub fn phase(&self) -> LifecyclePhase {
        self.phase
    }
}

/// Static builder contract implemented by every generated named application host.
pub trait AppHost {
    /// The single protocol plugin configured by this host.
    type Protocol: ProtocolPlugin;

    /// Creates a fresh protocol-specific application builder.
    fn builder() -> Result<AppBuilder<Self::Protocol>, overseerd_config::ConfigError>;
}

#[cfg(test)]
mod tests;
