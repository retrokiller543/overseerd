//! Shared paths emitted by the macros.
//!
//! Generated code intentionally imports framework items from the facade crate,
//! not the implementation crates. If the facade crate name changes, update this
//! constant and the expansions stay in sync.

pub(crate) const OVERSEERD_CRATE: &str = "overseerd";

/// A path to a **core-framework** item, rooted at the always-present `overseerd` facade.
/// Used for vocabulary, the DI engine, config, hooks, and transport — everything any plugin
/// can rely on.
pub(crate) fn overseerd_path(item: &str) -> syn::Path {
    syn::parse_str(&format!("::{OVERSEERD_CRATE}::{item}"))
        .expect("valid overseerd facade item path")
}

/// A path to a **daemon (RPC) plugin** item, rooted at the facade's `daemon` module
/// (`::overseerd::daemon::<item>`). The daemon macros (`#[service]`/`#[handlers]`/`#[rpc]`/
/// `app!`) emit their RPC-specific types this way, so end users depend only on `overseerd`
/// while the items stay namespaced under `daemon` — leaving the facade root free for other
/// plugins. (A future macro change will let a standalone/3rd-party plugin override this root.)
pub(crate) fn overseerd_daemon_path(item: &str) -> syn::Path {
    syn::parse_str(&format!("::{OVERSEERD_CRATE}::daemon::{item}"))
        .expect("valid overseerd daemon item path")
}
