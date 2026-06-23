use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServiceManagerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("service manager command failed (exit {code}): {stderr}")]
    CommandFailed { code: i32, stderr: String },

    #[error("service manager is not available on this platform ({platform})")]
    Unsupported { platform: &'static str },

    #[error("service is not installed; run `install` first")]
    NotInstalled,

    #[error("failed to determine current executable path: {0}")]
    ExecutablePath(std::io::Error),

    #[error("failed to determine home directory")]
    HomeDir,
}

pub type Result<T, E = ServiceManagerError> = std::result::Result<T, E>;
