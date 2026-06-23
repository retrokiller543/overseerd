pub mod backend;
pub mod builder;
pub mod cli;
pub mod error;
pub mod manager;
pub mod platform;

pub use builder::{ServiceConfig, ServiceManagerBuilder};
pub use cli::ServiceCommand;
pub use error::{ServiceManagerError, Result};
pub use manager::{ServiceManager, ServiceScope, ServiceStatus};
