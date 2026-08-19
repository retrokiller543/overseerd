//! Feature-gated generated CLI bootstrap and dispatch primitives.

mod bootstrap;
mod clap;
mod command;
#[cfg(feature = "tooling")]
pub(crate) mod metadata;
mod plugin;
mod types;

pub use bootstrap::*;
pub use clap::*;
pub use command::*;
pub use plugin::*;
pub use types::*;

#[cfg(test)]
mod tests;
