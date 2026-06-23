use crate::{ServiceScope, backend::ServiceManagerBackend};

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
pub mod windows;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub mod unsupported;

/// Selects the appropriate backend for the current platform and scope.
pub fn select_backend(scope: &ServiceScope) -> Box<dyn ServiceManagerBackend> {
    #[cfg(target_os = "linux")]
    return Box::new(linux::SystemdBackend::new(scope));

    #[cfg(target_os = "macos")]
    return Box::new(macos::LaunchdBackend::new(scope));

    #[cfg(windows)]
    return Box::new(windows::WindowsScmBackend::new(scope));

    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    return Box::new(unsupported::UnsupportedBackend);
}
