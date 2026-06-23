use std::path::PathBuf;

use crate::{ServiceManager, ServiceScope, error::Result};

/// Configuration for a managed service.
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub name: String,
    pub description: String,
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub working_dir: Option<PathBuf>,
    pub scope: ServiceScope,
    pub restart_on_failure: bool,
    /// Watchdog timeout written into the unit file (systemd `WatchdogSec`).
    /// Set this when the daemon uses the watchdog protocol.
    pub watchdog_sec: Option<u64>,
}

pub struct ServiceManagerBuilder {
    config: ServiceConfig,
}

impl ServiceManagerBuilder {
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self {
            config: ServiceConfig {
                name: name.into(),
                description: String::new(),
                executable: PathBuf::new(),
                args: Vec::new(),
                env: Vec::new(),
                working_dir: None,
                scope: ServiceScope::User,
                restart_on_failure: true,
                watchdog_sec: None,
            },
        }
    }

    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.config.description = desc.into();
        self
    }

    /// Sets the path to the daemon executable.
    /// Defaults to `std::env::current_exe()` if not set.
    pub fn executable(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.executable = path.into();
        self
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.config.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn env(mut self, env: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>) -> Self {
        self.config.env = env.into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        self
    }

    pub fn working_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.working_dir = Some(path.into());
        self
    }

    /// Whether to install as a user service (`~/.config/systemd/user/`,
    /// `~/Library/LaunchAgents/`) or a system service (`/etc/systemd/system/`,
    /// `/Library/LaunchDaemons/`). Defaults to `User`.
    pub fn scope(mut self, scope: ServiceScope) -> Self {
        self.config.scope = scope;
        self
    }

    pub fn restart_on_failure(mut self, restart: bool) -> Self {
        self.config.restart_on_failure = restart;
        self
    }

    /// Configures the watchdog timeout in seconds. Set this when the daemon
    /// uses the watchdog ping protocol so the service manager can restart it
    /// if it becomes unresponsive.
    pub fn watchdog_sec(mut self, secs: u64) -> Self {
        self.config.watchdog_sec = Some(secs);
        self
    }

    pub fn build(mut self) -> Result<ServiceManager> {
        if self.config.executable.as_os_str().is_empty() {
            self.config.executable =
                std::env::current_exe().map_err(crate::error::ServiceManagerError::ExecutablePath)?;
        }

        if self.config.description.is_empty() {
            self.config.description = self.config.name.clone();
        }

        let backend = crate::platform::select_backend(&self.config.scope);
        Ok(ServiceManager {
            config: self.config,
            backend,
        })
    }
}
