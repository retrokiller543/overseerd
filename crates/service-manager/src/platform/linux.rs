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

pub struct SystemdBackend {
    scope: ServiceScope,
}

impl SystemdBackend {
    pub fn new(scope: &ServiceScope) -> Self {
        Self { scope: *scope }
    }

    fn unit_dir(&self) -> Result<PathBuf> {
        match self.scope {
            ServiceScope::User => {
                let home = home_dir()?;
                Ok(home.join(".config/systemd/user"))
            }
            ServiceScope::System => Ok(PathBuf::from("/etc/systemd/system")),
        }
    }

    fn unit_path(&self, name: &str) -> Result<PathBuf> {
        Ok(self.unit_dir()?.join(format!("{name}.service")))
    }

    fn systemctl(&self, args: &[&str]) -> Result<()> {
        let mut cmd = Command::new("systemctl");
        if matches!(self.scope, ServiceScope::User) {
            cmd.arg("--user");
        }
        cmd.args(args);

        let output = cmd.output()?;

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(ServiceManagerError::CommandFailed { code, stderr });
        }

        Ok(())
    }

    fn generate_unit(&self, config: &ServiceConfig) -> String {
        let exe = config.executable.display();
        let args = if config.args.is_empty() {
            String::new()
        } else {
            format!(" {}", config.args.join(" "))
        };

        let env_lines: String = config
            .env
            .iter()
            .map(|(k, v)| format!("Environment={k}={v}\n"))
            .collect();

        let working_dir = config
            .working_dir
            .as_ref()
            .map(|p| format!("WorkingDirectory={}\n", p.display()))
            .unwrap_or_default();

        let restart = if config.restart_on_failure {
            "Restart=on-failure\n"
        } else {
            ""
        };

        let watchdog = config
            .watchdog_sec
            .map(|s| format!("WatchdogSec={s}s\n"))
            .unwrap_or_default();

        format!(
            "[Unit]\n\
             Description={description}\n\
             \n\
             [Service]\n\
             Type=notify\n\
             ExecStart={exe}{args}\n\
             {env_lines}{working_dir}{restart}{watchdog}\
             \n\
             [Install]\n\
             WantedBy=default.target\n",
            description = config.description,
        )
    }
}

impl ServiceManagerBackend for SystemdBackend {
    fn install(&self, config: &ServiceConfig) -> Result<()> {
        let unit_dir = self.unit_dir()?;
        fs::create_dir_all(&unit_dir)?;

        let unit_content = self.generate_unit(config);
        let unit_path = self.unit_path(&config.name)?;
        fs::write(&unit_path, unit_content)?;

        self.systemctl(&["daemon-reload"])
    }

    fn uninstall(&self, config: &ServiceConfig) -> Result<()> {
        let _ = self.systemctl(&["stop", &config.name]);
        let _ = self.systemctl(&["disable", &config.name]);

        let unit_path = self.unit_path(&config.name)?;
        if unit_path.exists() {
            fs::remove_file(unit_path)?;
        }

        self.systemctl(&["daemon-reload"])
    }

    fn start(&self, name: &str) -> Result<()> {
        self.systemctl(&["start", name])
    }

    fn stop(&self, name: &str) -> Result<()> {
        self.systemctl(&["stop", name])
    }

    fn restart(&self, name: &str) -> Result<()> {
        self.systemctl(&["restart", name])
    }

    fn enable(&self, name: &str) -> Result<()> {
        self.systemctl(&["enable", name])
    }

    fn disable(&self, name: &str) -> Result<()> {
        self.systemctl(&["disable", name])
    }

    fn status(&self, name: &str) -> Result<ServiceStatus> {
        let mut cmd = Command::new("systemctl");
        if matches!(self.scope, ServiceScope::User) {
            cmd.arg("--user");
        }
        cmd.args(["is-active", name]);

        let output = cmd.output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let state = stdout.trim();

        Ok(match state {
            "active" => ServiceStatus::Running,
            "inactive" | "dead" => ServiceStatus::Stopped,
            "failed" => ServiceStatus::Failed,
            other => ServiceStatus::Unknown(other.to_owned()),
        })
    }

    fn is_installed(&self, name: &str) -> bool {
        self.unit_path(name).map(|p| p.exists()).unwrap_or(false)
    }
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(ServiceManagerError::HomeDir)
}
