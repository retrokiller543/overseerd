//! Target-conditional gating of generated code.

use proc_macro2::TokenStream;
use quote::quote;

/// Prefixes `#[cfg(not(target_family = "wasm"))]` to every top-level item in `tokens`, compiling
/// the whole generated group only on non-wasm targets.
///
/// Server-side generated code — DI wiring, `linkme` registration, route tables, the user's own
/// `struct`/`impl` bodies — references framework paths and the `linkme` slices that do not exist on
/// a wasm client build. Gating it lets one shared `#[controller]`/`#[service]`/`#[config]` crate
/// compile *both* natively (server) and to wasm (client), where only the generated client survives.
///
/// Each top-level item is gated individually (a `linkme` `distributed_slice` declaration must stay
/// top-level, and a single `#[cfg]` only covers the next item), so the input is re-parsed as a
/// sequence of items. Natively the predicate is always true, so the output is byte-identical to the
/// input. An empty stream stays empty (no dangling attribute).
pub fn native_only(tokens: TokenStream) -> TokenStream {
    if tokens.is_empty() {
        return tokens;
    }

    let file: syn::File =
        syn::parse2(tokens).expect("generated server code is a valid sequence of items");
    let items = file.items.into_iter().map(|item| {
        quote! {
            #[cfg(not(target_family = "wasm"))]
            #item
        }
    });

    quote! {
        #(#items)*
    }
}
