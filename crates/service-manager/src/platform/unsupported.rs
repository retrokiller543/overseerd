use crate::{
    ServiceStatus,
    backend::ServiceManagerBackend,
    builder::ServiceConfig,
    error::{Result, ServiceManagerError},
};

pub struct UnsupportedBackend;

const PLATFORM: &str = std::env::consts::OS;

impl ServiceManagerBackend for UnsupportedBackend {
    fn install(&self, _: &ServiceConfig) -> Result<()> {
        Err(ServiceManagerError::Unsupported { platform: PLATFORM })
    }

    fn uninstall(&self, _: &ServiceConfig) -> Result<()> {
        Err(ServiceManagerError::Unsupported { platform: PLATFORM })
    }

    fn start(&self, _: &str) -> Result<()> {
        Err(ServiceManagerError::Unsupported { platform: PLATFORM })
    }

    fn stop(&self, _: &str) -> Result<()> {
        Err(ServiceManagerError::Unsupported { platform: PLATFORM })
    }

    fn restart(&self, _: &str) -> Result<()> {
        Err(ServiceManagerError::Unsupported { platform: PLATFORM })
    }

    fn enable(&self, _: &str) -> Result<()> {
        Err(ServiceManagerError::Unsupported { platform: PLATFORM })
    }

    fn disable(&self, _: &str) -> Result<()> {
        Err(ServiceManagerError::Unsupported { platform: PLATFORM })
    }

    fn status(&self, _: &str) -> Result<ServiceStatus> {
        Err(ServiceManagerError::Unsupported { platform: PLATFORM })
    }

    fn is_installed(&self, _: &str) -> bool {
        false
    }
}
