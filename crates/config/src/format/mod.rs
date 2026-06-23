//! Source-format parsers that normalize into the shared [`ConfigValue`](crate::ConfigValue)
//! tree, each behind its own cargo feature. Placeholder parsing happens here, per
//! string leaf, so everything downstream is format-agnostic.

#[cfg(feature = "toml")]
pub mod toml;
#[cfg(feature = "yaml")]
pub mod yaml;
