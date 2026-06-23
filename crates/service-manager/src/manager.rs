use crate::{backend::ServiceManagerBackend, builder::ServiceConfig, error::Result};

/// Whether the service is installed as a user service or a system service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum ServiceScope {
    /// User-level service (`~/.config/systemd/user/` on Linux,
    /// `~/Library/LaunchAgents/` on macOS). Does not require root.
    #[default]
    User,
    /// System-level service (`/etc/systemd/system/` on Linux,
    /// `/Library/LaunchDaemons/` on macOS). Requires root.
    System,
}

/// Current runtime status of the managed service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceStatus {
    Running,
    Stopped,
    Failed,
    Unknown(String),
}

/// Manages the system service lifecycle for an Overseerd daemon.
///
/// Wraps a platform-specific backend (systemd, launchd, Windows SCM, …) and
/// exposes a uniform install/uninstall/start/stop API. Constructed via
/// [`ServiceManager::new`] (returns a builder).
pub struct ServiceManager {
    pub(crate) config: ServiceConfig,
    pub(crate) backend: Box<dyn ServiceManagerBackend>,
}

impl ServiceManager {
    pub fn new(name: impl Into<String>) -> crate::builder::ServiceManagerBuilder {
        crate::builder::ServiceManagerBuilder::new(name)
    }

    /// Installs the service (writes the unit file / plist and reloads the daemon).
    pub fn install(&self) -> Result<()> {
        self.backend.install(&self.config)
    }

    /// Removes the service file and unregisters it from the service manager.
    pub fn uninstall(&self) -> Result<()> {
        self.backend.uninstall(&self.config)
    }

    /// Starts the installed service.
    pub fn start(&self) -> Result<()> {
        self.backend.start(&self.config.name)
    }

    /// Stops the running service.
    pub fn stop(&self) -> Result<()> {
        self.backend.stop(&self.config.name)
    }

    /// Restarts the service (stop then start).
    pub fn restart(&self) -> Result<()> {
        self.backend.restart(&self.config.name)
    }

    /// Enables the service to start automatically on boot.
    pub fn enable(&self) -> Result<()> {
        self.backend.enable(&self.config.name)
    }

    /// Disables automatic startup on boot.
    pub fn disable(&self) -> Result<()> {
        self.backend.disable(&self.config.name)
    }

    /// Returns the current status of the service.
    pub fn status(&self) -> Result<ServiceStatus> {
        self.backend.status(&self.config.name)
    }

    /// Returns `true` if the service unit / plist file is present.
    pub fn is_installed(&self) -> bool {
        self.backend.is_installed(&self.config.name)
    }
}
