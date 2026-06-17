//! Standalone `#[rpc]` expansion.
//!
//! `#[rpc]` is normally stripped by `#[service]` before the compiler resolves
//! it, so reaching here means it was used outside a `#[service]` impl. We emit
//! the original function unchanged (so its symbol still exists and downstream
//! code doesn't cascade) alongside a clear error — the fallback pattern used by
//! `thiserror-impl`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::ItemFn;

pub fn expand_standalone(item: ItemFn) -> syn::Result<TokenStream> {
    let error = syn::Error::new(
        item.sig.ident.span(),
        "`#[rpc]` must be applied to a method inside a `#[service]` impl block",
    )
    .into_compile_error();

    Ok(quote! {
        #item
        #error
    })
}
