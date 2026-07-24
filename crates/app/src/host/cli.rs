//! Feature-gated generated CLI bootstrap and dispatch primitives.

mod bootstrap;
mod types;

pub use bootstrap::*;
pub use types::*;

#[cfg(test)]
mod tests;
