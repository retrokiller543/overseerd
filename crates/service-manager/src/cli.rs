use crate::{ServiceManager, ServiceScope, error::Result};

/// Service lifecycle subcommands to embed in your daemon's clap CLI.
///
/// # Example
///
/// ```rust,ignore
/// #[derive(clap::Parser)]
/// struct Cli {
///     #[command(subcommand)]
///     command: Option<Command>,
/// }
///
/// #[derive(clap::Subcommand)]
/// enum Command {
///     #[command(flatten)]
///     Service(overseerd::service::ServiceCommand),
/// }
///
/// fn main() -> anyhow::Result<()> {
///     let cli = Cli::parse();
///     if let Some(Command::Service(cmd)) = cli.command {
///         let manager = ServiceManager::new("my-daemon").build()?;
///         cmd.execute(&manager)?;
///         return Ok(());
///     }
///     // ... run the daemon ...
/// }
/// ```
#[derive(Debug, clap::Subcommand)]
pub enum ServiceCommand {
    /// Install the daemon as a system service.
    Install {
        /// Install as a user service (no root required) or a system service.
        #[arg(long, default_value = "user")]
        scope: ServiceScope,
        /// Watchdog timeout in seconds. Set when the daemon uses the watchdog
        /// ping protocol so the service manager can restart it if unresponsive.
        #[arg(long)]
        watchdog_sec: Option<u64>,
    },

    /// Remove the installed service.
    Uninstall {
        #[arg(long, default_value = "user")]
        scope: ServiceScope,
    },

    /// Start the installed service.
    Start,

    /// Stop the running service.
    Stop,

    /// Restart the service.
    Restart,

    /// Enable the service to start automatically on boot.
    Enable,

    /// Disable automatic startup on boot.
    Disable,

    /// Show the current service status.
    Status,

    /// Run the daemon directly in the foreground (no service manager).
    ///
    /// Use this for local development or when you want to manage the process
    /// yourself. When omitted, most applications default to this behaviour.
    Run,
}

impl ServiceCommand {
    /// Executes the service command using the given `ServiceManager`.
    ///
    /// Returns `Ok(true)` when the command was handled (the caller should
    /// return), or `Ok(false)` for the `Run` variant (the caller should
    /// proceed to start the daemon in-process).
    pub fn execute(self, manager: &ServiceManager) -> Result<bool> {
        match self {
            ServiceCommand::Install { scope: _, watchdog_sec: _ } => {
                manager.install()?;
                println!("Service '{}' installed.", manager.config.name);
                println!("Run `{} start` to start it.", manager.config.name);
                Ok(true)
            }

            ServiceCommand::Uninstall { scope: _ } => {
                manager.uninstall()?;
                println!("Service '{}' uninstalled.", manager.config.name);
                Ok(true)
            }

            ServiceCommand::Start => {
                manager.start()?;
                println!("Service '{}' started.", manager.config.name);
                Ok(true)
            }

            ServiceCommand::Stop => {
                manager.stop()?;
                println!("Service '{}' stopped.", manager.config.name);
                Ok(true)
            }

            ServiceCommand::Restart => {
                manager.restart()?;
                println!("Service '{}' restarted.", manager.config.name);
                Ok(true)
            }

            ServiceCommand::Enable => {
                manager.enable()?;
                println!("Service '{}' enabled for automatic startup.", manager.config.name);
                Ok(true)
            }

            ServiceCommand::Disable => {
                manager.disable()?;
                println!("Service '{}' disabled from automatic startup.", manager.config.name);
                Ok(true)
            }

            ServiceCommand::Status => {
                let status = manager.status()?;
                println!("{:?}", status);
                Ok(true)
            }

            ServiceCommand::Run => Ok(false),
        }
    }
}
