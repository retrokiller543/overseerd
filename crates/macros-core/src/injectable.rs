//! `#[injectable]` expansion (trait): marks a trait as injectable as
//! `Arc<dyn Trait>`.
//!
//! Under `di-check` it emits `impl Provide<dyn Trait> for Wiring` once, at the
//! trait, so a single `Arc<dyn Trait>` dependency type-checks regardless of how
//! many components `provide = dyn Trait` it (no per-provider `E0119`). Provider
//! existence and ambiguity are checked at runtime and by the source analyzer.
//! Without `di-check`, the trait passes through unchanged.

use proc_macro2::TokenStream;
use quote::quote;
use syn::ItemTrait;

use crate::di;
use crate::paths::Paths;

pub fn expand(item: ItemTrait, paths: &Paths) -> TokenStream {
    let provide = di::injectable_impl(&item.ident, paths);

    quote! {
        #item

        #provide
    }
}
