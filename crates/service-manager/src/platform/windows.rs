use crate::{
    ServiceScope, ServiceStatus,
    backend::ServiceManagerBackend,
    builder::ServiceConfig,
    error::{Result, ServiceManagerError},
};

pub struct WindowsScmBackend {
    #[allow(dead_code)]
    scope: ServiceScope,
}

impl WindowsScmBackend {
    pub fn new(scope: &ServiceScope) -> Self {
        Self { scope: *scope }
    }
}

impl ServiceManagerBackend for WindowsScmBackend {
    fn install(&self, config: &ServiceConfig) -> Result<()> {
        use windows_service::{
            service::{ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceType},
            service_manager::{ServiceManager, ServiceManagerAccess},
        };

        let manager = ServiceManager::local_computer(
            None::<&str>,
            ServiceManagerAccess::CREATE_SERVICE,
        )?;

        let service_info = ServiceInfo {
            name: std::ffi::OsString::from(&config.name),
            display_name: std::ffi::OsString::from(&config.description),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::OnDemand,
            error_control: ServiceErrorControl::Normal,
            executable_path: config.executable.clone(),
            launch_arguments: config.args.iter().map(std::ffi::OsString::from).collect(),
            dependencies: vec![],
            account_name: None,
            account_password: None,
        };

        manager.create_service(&service_info, ServiceAccess::all())?;
        Ok(())
    }

    fn uninstall(&self, config: &ServiceConfig) -> Result<()> {
        use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
        let service = manager.open_service(
            &config.name,
            windows_service::service::ServiceAccess::DELETE,
        )?;
        service.delete()?;
        Ok(())
    }

    fn start(&self, name: &str) -> Result<()> {
        with_service(name, windows_service::service::ServiceAccess::START, |svc| {
            svc.start(&[] as &[&std::ffi::OsStr])?;
            Ok(())
        })
    }

    fn stop(&self, name: &str) -> Result<()> {
        use windows_service::service::ServiceControl;
        with_service(name, windows_service::service::ServiceAccess::STOP, |svc| {
            svc.control(ServiceControl::Stop)?;
            Ok(())
        })
    }

    fn restart(&self, name: &str) -> Result<()> {
        self.stop(name)?;
        self.start(name)
    }

    fn enable(&self, name: &str) -> Result<()> {
        use windows_service::service::{ServiceAccess, ServiceStartType};
        with_service(name, ServiceAccess::CHANGE_CONFIG, |svc| {
            svc.change_config()?.start_type(ServiceStartType::AutoStart).revert();
            Ok(())
        })
    }

    fn disable(&self, name: &str) -> Result<()> {
        use windows_service::service::{ServiceAccess, ServiceStartType};
        with_service(name, ServiceAccess::CHANGE_CONFIG, |svc| {
            svc.change_config()?.start_type(ServiceStartType::Disabled).revert();
            Ok(())
        })
    }

    fn status(&self, name: &str) -> Result<ServiceStatus> {
        use windows_service::service::{ServiceAccess, ServiceState};
        with_service(name, ServiceAccess::QUERY_STATUS, |svc| {
            let status = svc.query_status()?;
            Ok(match status.current_state {
                ServiceState::Running => ServiceStatus::Running,
                ServiceState::Stopped => ServiceStatus::Stopped,
                _ => ServiceStatus::Unknown(format!("{:?}", status.current_state)),
            })
        })
    }

    fn is_installed(&self, name: &str) -> bool {
        use windows_service::{
            service::ServiceAccess,
            service_manager::{ServiceManager, ServiceManagerAccess},
        };
        let Ok(manager) =
            ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        else {
            return false;
        };
        manager
            .open_service(name, ServiceAccess::QUERY_STATUS)
            .is_ok()
    }
}

fn with_service<F, T>(
    name: &str,
    access: windows_service::service::ServiceAccess,
    f: F,
) -> Result<T>
where
    F: FnOnce(windows_service::service::Service) -> Result<T>,
{
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(name, access)?;
    f(service)
}

impl From<windows_service::Error> for ServiceManagerError {
    fn from(e: windows_service::Error) -> Self {
        ServiceManagerError::Io(std::io::Error::other(e.to_string()))
    }
}
