//! Feature-gated generated CLI bootstrap and dispatch primitives.

mod bootstrap;
mod clap;
mod command;
mod types;

pub use bootstrap::*;
pub use clap::*;
pub use command::*;
pub use types::*;

#[cfg(test)]
mod tests;
