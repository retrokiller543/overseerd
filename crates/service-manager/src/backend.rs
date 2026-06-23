use crate::{ServiceStatus, builder::ServiceConfig, error::Result};

/// Platform-specific implementation of service management operations.
///
/// Implement this trait to add support for additional service managers
/// (e.g. runit, s6, OpenRC). The `ServiceManager` type dispatches all
/// operations to whichever backend was selected at construction time.
pub trait ServiceManagerBackend: Send + Sync {
    /// Installs the service (writes unit file / plist, reloads daemon).
    fn install(&self, config: &ServiceConfig) -> Result<()>;

    /// Removes the service (stops if running, disables, removes the file).
    fn uninstall(&self, config: &ServiceConfig) -> Result<()>;

    /// Starts the installed service.
    fn start(&self, name: &str) -> Result<()>;

    /// Stops the running service.
    fn stop(&self, name: &str) -> Result<()>;

    /// Stops then starts the service.
    fn restart(&self, name: &str) -> Result<()>;

    /// Enables the service to start automatically on boot.
    fn enable(&self, name: &str) -> Result<()>;

    /// Disables automatic startup on boot.
    fn disable(&self, name: &str) -> Result<()>;

    /// Returns the current status of the service.
    fn status(&self, name: &str) -> Result<ServiceStatus>;

    /// Returns `true` if the service unit / plist is present on disk.
    fn is_installed(&self, name: &str) -> bool;
}
