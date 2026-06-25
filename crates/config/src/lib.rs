//! Type-safe configuration value substitution.
//!
//! This crate is the format-agnostic core of Overseerd's config system: a normalized
//! [`ConfigValue`] tree, a placeholder grammar (`${KEY}` / `${KEY:default}`), a
//! [`Resolver`] chain, and a custom [`from_value`] deserializer that resolves
//! placeholders *while* deserializing — so a leaf that is entirely `${VAR}` can become
//! any scalar the target type wants, while a templated leaf like `https://${VAR}` can
//! only ever be a string. It owns no I/O, merging, or file-watching; those layers sit
//! on top of [`from_value`].

mod de;
mod defaults;
mod error;
pub mod format;
mod parse;
mod resolve;
mod value;

pub use de::{ValueDeserializer, from_value, from_value_in};
pub use defaults::{DefaultSpec, EnumTag};
pub use error::{ConfigError, ConfigErrorKind};
pub use resolve::{EnvResolver, MapResolver, ResolveCtx, Resolver, ResolverChain};
pub use value::{ConfigStr, ConfigValue, Placeholder, Segment};
