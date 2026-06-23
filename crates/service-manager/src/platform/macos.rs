use std::{
    fs,
    path::PathBuf,
    process::Command,
};

use crate::{
    ServiceScope, ServiceStatus,
    backend::ServiceManagerBackend,
    builder::ServiceConfig,
    error::{Result, ServiceManagerError},
};

pub struct LaunchdBackend {
    scope: ServiceScope,
}

impl LaunchdBackend {
    pub fn new(scope: &ServiceScope) -> Self {
        Self { scope: *scope }
    }

    /// Reverse-domain label: `com.<username>.<name>`.
    fn label(&self, name: &str) -> String {
        let username = std::env::var("USER").unwrap_or_else(|_| "user".to_owned());
        format!("com.{username}.{name}")
    }

    fn plist_dir(&self) -> Result<PathBuf> {
        match self.scope {
            ServiceScope::User => {
                let home = home_dir()?;
                Ok(home.join("Library/LaunchAgents"))
            }
            ServiceScope::System => Ok(PathBuf::from("/Library/LaunchDaemons")),
        }
    }

    fn plist_path(&self, name: &str) -> Result<PathBuf> {
        Ok(self.plist_dir()?.join(format!("{}.plist", self.label(name))))
    }

    fn generate_plist(&self, config: &ServiceConfig) -> String {
        let label = self.label(&config.name);
        let exe = config.executable.display();

        let args_xml = if config.args.is_empty() {
            String::new()
        } else {
            config
                .args
                .iter()
                .map(|a| format!("\t\t<string>{a}</string>\n"))
                .collect::<String>()
        };

        let env_xml = if config.env.is_empty() {
            String::new()
        } else {
            let pairs: String = config
                .env
                .iter()
                .map(|(k, v)| format!("\t\t<key>{k}</key>\n\t\t<string>{v}</string>\n"))
                .collect();
            format!("\t<key>EnvironmentVariables</key>\n\t<dict>\n{pairs}\t</dict>\n")
        };

        let working_dir_xml = config
            .working_dir
            .as_ref()
            .map(|p| format!("\t<key>WorkingDirectory</key>\n\t<string>{}</string>\n", p.display()))
            .unwrap_or_default();

        let keepalive = if config.restart_on_failure {
            "\t<key>KeepAlive</key>\n\t<true/>\n"
        } else {
            "\t<key>KeepAlive</key>\n\t<false/>\n"
        };

        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
             \t<key>Label</key>\n\
             \t<string>{label}</string>\n\
             \t<key>ProgramArguments</key>\n\
             \t<array>\n\
             \t\t<string>{exe}</string>\n\
             {args_xml}\
             \t</array>\n\
             {env_xml}\
             {working_dir_xml}\
             {keepalive}\
             \t<key>RunAtLoad</key>\n\
             \t<false/>\n\
             </dict>\n\
             </plist>\n"
        )
    }
}

impl ServiceManagerBackend for LaunchdBackend {
    fn install(&self, config: &ServiceConfig) -> Result<()> {
        let plist_dir = self.plist_dir()?;
        fs::create_dir_all(&plist_dir)?;

        let plist = self.generate_plist(config);
        let plist_path = self.plist_path(&config.name)?;
        fs::write(&plist_path, plist)?;

        Ok(())
    }

    fn uninstall(&self, config: &ServiceConfig) -> Result<()> {
        let plist_path = self.plist_path(&config.name)?;
        if plist_path.exists() {
            let _ = Command::new("launchctl")
                .args(["unload", &plist_path.to_string_lossy()])
                .output();
            fs::remove_file(plist_path)?;
        }
        Ok(())
    }

    fn start(&self, name: &str) -> Result<()> {
        let label = self.label(name);
        let output = Command::new("launchctl").args(["start", &label]).output()?;
        check_launchctl(output)
    }

    fn stop(&self, name: &str) -> Result<()> {
        let label = self.label(name);
        let output = Command::new("launchctl").args(["stop", &label]).output()?;
        check_launchctl(output)
    }

    fn restart(&self, name: &str) -> Result<()> {
        self.stop(name)?;
        self.start(name)
    }

    fn enable(&self, name: &str) -> Result<()> {
        let plist_path = self.plist_path(name)?;
        if !plist_path.exists() {
            return Err(ServiceManagerError::NotInstalled);
        }
        let output = Command::new("launchctl")
            .args(["load", "-w", &plist_path.to_string_lossy()])
            .output()?;
        check_launchctl(output)
    }

    fn disable(&self, name: &str) -> Result<()> {
        let plist_path = self.plist_path(name)?;
        let output = Command::new("launchctl")
            .args(["unload", "-w", &plist_path.to_string_lossy()])
            .output()?;
        check_launchctl(output)
    }

    fn status(&self, name: &str) -> Result<ServiceStatus> {
        let label = self.label(name);
        let output = Command::new("launchctl").args(["list", &label]).output()?;

        if !output.status.success() {
            return Ok(ServiceStatus::Stopped);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        // launchctl list output contains "PID" = <number> when running
        let running = stdout.lines().any(|line| {
            let trimmed = line.trim();
            trimmed.starts_with("\"PID\"")
                && !trimmed.contains("0")
                && !trimmed.ends_with("0;")
        });

        Ok(if running {
            ServiceStatus::Running
        } else {
            ServiceStatus::Stopped
        })
    }

    fn is_installed(&self, name: &str) -> bool {
        self.plist_path(name).map(|p| p.exists()).unwrap_or(false)
    }
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(ServiceManagerError::HomeDir)
}

fn check_launchctl(output: std::process::Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    let code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Err(ServiceManagerError::CommandFailed { code, stderr })
}
