//! Optional compile-time dependency-injection checks, emitted only under the
//! `di-check` feature.
//!
//! Each component registers itself with `impl Provide<Self> for Wiring`, and each
//! field-injected component asserts its concrete dependencies with an uncalled
//! `fn _() where Wiring: Provide<Dep> { }` — whose bound the compiler discharges
//! at definition, so a missing provider is a `cargo check` error. Trait-object
//! (`Arc<dyn Trait>`) dependencies are intentionally not asserted here: the
//! "single provider" rule needs a whole-graph view (see the `app!` design); the
//! build-time source analyzer covers them instead.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

use crate::paths::overseerd_path;

/// Whether compile-time DI checking is enabled for this build.
pub fn enabled() -> bool {
    cfg!(feature = "di-check")
}

/// `impl Provide<Self> for Wiring {}`, registering a component as a provider of
/// its own concrete type. Empty unless `di-check` is on.
pub fn provide_impl(self_ident: &Ident) -> TokenStream {
    if !enabled() {
        return quote!();
    }

    let provide = overseerd_path("Provide");
    let wiring = overseerd_path("Wiring");

    quote! {
        impl #provide<#self_ident> for #wiring {}
    }
}

/// `impl Wired for T where Wiring: Provide<T1> + .. {}` — a lazy predicate that
/// all of `T`'s single dependencies (`targets`, concrete and trait-object) are
/// provided, checked only where `app!` demands `T: Wired`. A type with no
/// dependencies is unconditionally `Wired`. Empty unless `di-check` is on.
pub fn wired_impl(self_ident: &Ident, targets: &[TokenStream]) -> TokenStream {
    if !enabled() {
        return quote!();
    }

    let wired = overseerd_path("Wired");
    let provide = overseerd_path("Provide");
    let wiring = overseerd_path("Wiring");

    if targets.is_empty() {
        quote! {
            impl #wired for #self_ident {}
        }
    } else {
        quote! {
            impl #wired for #self_ident
            where
                #wiring: #(#provide<#targets>)+*,
            {
            }
        }
    }
}

/// `impl Provide<dyn Trait> for Wiring {}`, emitted once at the trait's
/// definition by `#[injectable]`, so a single `Arc<dyn Trait>` dependency
/// type-checks. Living on the trait (not each provider) means it is coherent no
/// matter how many components `provide` it — provider *existence* is checked at
/// runtime and by the source analyzer. Empty unless `di-check` is on.
pub fn injectable_impl(trait_ident: &Ident) -> TokenStream {
    if !enabled() {
        return quote!();
    }

    let provide = overseerd_path("Provide");
    let wiring = overseerd_path("Wiring");

    quote! {
        impl #provide<dyn #trait_ident> for #wiring {}
    }
}

/// An uncalled assertion that every `target` is provided, as the bound
/// `Wiring: Provide<T1> + Provide<T2> + ..`. Each `target` is a `Provide` type
/// argument (e.g. `<Arc<T> as Injectable>::Target`). Empty unless `di-check` is
/// on or there are no targets.
pub fn assert(targets: &[TokenStream]) -> TokenStream {
    if !enabled() || targets.is_empty() {
        return quote!();
    }

    let provide = overseerd_path("Provide");
    let wiring = overseerd_path("Wiring");

    quote! {
        const _: () = {
            fn __overseerd_assert_di()
            where
                #wiring: #(#provide<#targets>)+*,
            {
            }
        };
    }
}
