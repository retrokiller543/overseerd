//! Shared paths emitted by the macros.
//!
//! Generated code intentionally imports framework items from the facade crate,
//! not the implementation crates. If the facade crate name changes, update this
//! constant and the expansions stay in sync.

pub(crate) const OVERSEERD_CRATE: &str = "overseerd";

pub(crate) fn overseerd_path(item: &str) -> syn::Path {
    syn::parse_str(&format!("::{OVERSEERD_CRATE}::{item}"))
        .expect("valid overseerd facade item path")
}
