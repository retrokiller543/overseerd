//! Registration-backend selection for generated code.
//!
//! Per-type metadata (component factories, hooks, RPC groups, routes) is registered either into a
//! `linkme` distributed slice (one custom linker section per per-type slice) or into an `inventory`
//! collection (one shared constructor section, section-count-independent). Apple/Mach-O targets cap
//! custom sections at ~255 per segment, so there the macros must emit the `inventory` variant.
//!
//! The choice is made **at expansion time** and only the chosen backend's tokens are emitted — no
//! `#[cfg]` leaks into the user's crate, so downstream builds need no cfg registration or lint
//! config. The signal is the `overseerd_hybrid` cfg (set by this crate's build script via
//! [`overseerd_build`](https://docs.rs/overseerd-build) — auto-on for the Apple/Mach-O host, or
//! forced with `OVERSEERD_HYBRID`) OR the `hybrid-registry` Cargo feature (the convenient force,
//! forwarded from the macro crates and the facade).

use proc_macro2::TokenStream;
use quote::quote;

use crate::paths::Paths;

/// Emits the `impl RegistryFor<D> for T` that anchors an `inventory` bucket for the `(owner, kind)`
/// pair — the orphan-legal shim (the owner type is local to the emitting crate) that the blanket
/// `inventory::Collect for DescriptorFor<T, D>` forwards to. One per owner/descriptor-kind; the
/// `static` inside its concrete `registry()` is the bucket, unique per monomorphization.
pub fn registry_for_impl(owner: TokenStream, kind: TokenStream, paths: &Paths) -> TokenStream {
    let registry_for = paths.core("RegistryFor");
    let inventory = paths.core("inventory");

    quote! {
        impl #registry_for<#kind> for #owner {
            fn registry() -> &'static #inventory::Registry {
                static REGISTRY: #inventory::Registry = #inventory::Registry::new();

                &REGISTRY
            }
        }
    }
}

/// Whether the macros should emit `inventory` registrations instead of `linkme` ones.
///
/// `target_os`/`overseerd_hybrid` here reflect the proc-macro **host** — correct for native builds;
/// cross-compiling to a Mach-O target from a non-Mach-O host needs the `hybrid-registry` feature.
#[inline]
pub fn use_inventory() -> bool {
    cfg!(overseerd_hybrid) || cfg!(feature = "hybrid-registry")
}

/// Selects between the two backends' token streams, emitting exactly one. `inventory` is chosen when
/// [`use_inventory`] holds, `linkme` otherwise.
#[inline]
pub fn dual_backend(inventory: TokenStream, linkme: TokenStream) -> TokenStream {
    if use_inventory() { inventory } else { linkme }
}
