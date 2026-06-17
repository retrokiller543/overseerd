//! Shared paths emitted by the macros.
//!
//! Generated code intentionally imports framework items from the facade crate,
//! not the implementation crates. If the facade crate name changes, update this
//! constant and the expansions stay in sync.

pub(crate) const OVERSEER_CRATE: &str = "overseer";

pub(crate) fn overseer_path(item: &str) -> syn::Path {
    syn::parse_str(&format!("::{OVERSEER_CRATE}::{item}")).expect("valid overseer facade item path")
}
