//! Shared codegen library for the Overseerd proc-macros.
//!
//! A proc-macro crate can only export proc-macros, so the reusable machinery lives here as
//! an ordinary library: [`Paths`] (crate-root resolution for generated code), [`ParseKeyed`]
//! (the extension seam for attribute arguments), and — added incrementally — the component /
//! hook / method models, descriptor-slice emission, and client generation. `overseerd-macros`
//! and each plugin's macro crate (`overseerd-rpc-macros`, …) are thin proc-macro shims over it.

mod parse_keyed;
mod paths;

pub use parse_keyed::{NoExt, ParseKeyed, eat_comma, eat_eq, unknown_key_error};
pub use paths::Paths;
