//! The `#[dto]` attribute: marks a type as HTTP wire data.
//!
//! It derives `serde::Serialize` + `serde::Deserialize` (unless `#[dto(no_serde)]`, for a type that
//! provides its own), derives `tsify::Tsify` on wasm (so the generated browser client is typed in
//! TypeScript), and implements [`Dto`](../overseerd_axum/trait.Dto.html) so the type may appear as a
//! handler's body, response, or path/query parameter.

use overseerd_macros_core::paths::Paths;
use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{DeriveInput, Ident, Token};

/// Parsed `#[dto(..)]` arguments.
#[derive(Default)]
pub struct DtoArgs {
    /// `no_serde` — skip the `serde` derives (the type provides its own `Serialize`/`Deserialize`).
    no_serde: bool,
}

impl Parse for DtoArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = DtoArgs::default();

        let idents = input.parse_terminated(Ident::parse, Token![,])?;

        for ident in idents {
            match ident.to_string().as_str() {
                "no_serde" => args.no_serde = true,

                other => {
                    return Err(syn::Error::new_spanned(
                        &ident,
                        format!("unknown `#[dto]` option `{other}` (expected `no_serde`)"),
                    ));
                }
            }
        }

        Ok(args)
    }
}

/// Expands `#[dto]` on a struct/enum: the derives (serde, and `Tsify` on wasm), the [`Dto`] impl,
/// and the type itself. `Tsify` is wasm-only (its ABI + TS generation are only meaningful for the
/// browser client), so a native server build pulls neither `tsify` nor `wasm-bindgen`.
pub fn expand(args: DtoArgs, item: DeriveInput, paths: &Paths) -> syn::Result<TokenStream> {
    let ident = &item.ident;
    let (impl_generics, ty_generics, where_clause) = item.generics.split_for_impl();
    let dto = paths.plugin("Dto");

    let serde_derive = if args.no_serde {
        quote!()
    } else {
        quote!(#[derive(::serde::Serialize, ::serde::Deserialize)])
    };

    // Typed wasm<->JS conversion + TypeScript generation, only for the browser client (its derive
    // hardcodes `::tsify`, so a wasm client crate depends on `tsify` directly). Two ABI flavours:
    //   - default: the published `tsify` — `#[tsify(into_wasm_abi, from_wasm_abi)]` makes the type
    //     usable directly as a `#[wasm_bindgen]` argument/return.
    //   - `wasm-ts`: the new `tsify` (git) — a plain `Tsify` derive; the client uses `Ts<T>` at the
    //     boundary instead (typed and free of the `into_wasm_abi` memory leak).
    let tsify_derive = if cfg!(feature = "wasm-ts") {
        quote!(#[cfg_attr(target_family = "wasm", derive(::tsify::Tsify))])
    } else {
        quote!(
            #[cfg_attr(
                target_family = "wasm",
                derive(::tsify::Tsify),
                tsify(into_wasm_abi, from_wasm_abi)
            )]
        )
    };

    // OpenAPI: a `utoipa::ToSchema` derive (so the type is a component schema) plus a link-time
    // registration into the schema slice. Gated inside the macro — `openapi_derive` is concrete
    // tokens or empty, never a `cfg_attr(feature = ..)` that would leak the feature onto the user's
    // crate (see the `crate::openapi` module docs).
    let (openapi_derive, openapi_registration) = crate::openapi::dto_tokens(&item, paths);

    Ok(quote! {
        #tsify_derive
        #serde_derive
        #openapi_derive
        #item

        impl #impl_generics #dto for #ident #ty_generics #where_clause {}

        #openapi_registration
    })
}
