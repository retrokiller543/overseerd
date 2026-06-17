//! `#[derive(Component)]` — implements the `Component` metadata trait for plain
//! dependency types (e.g. config, pools) so they can be registered via
//! `DaemonBuilder::with_component`.
//!
//! `ID` defaults to the lowercased type name and `NAME` to the type name; both
//! can be overridden with `#[component(id = "...", name = "...")]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, LitStr};

use crate::attr::ServiceArgs;

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let ident = &input.ident;

    let mut overrides = ServiceArgs {
        id: None,
        name: None,
        version: None,
    };

    for attr in &input.attrs {
        if attr.path().is_ident("component") {
            overrides = attr.parse_args::<ServiceArgs>()?;
        }
    }

    let id = overrides
        .id
        .unwrap_or_else(|| LitStr::new(&ident.to_string().to_lowercase(), ident.span()));
    let name = overrides
        .name
        .unwrap_or_else(|| LitStr::new(&ident.to_string(), ident.span()));

    Ok(quote! {
        impl ::overseer_core::Component for #ident {
            const ID: &'static str = #id;
            const NAME: &'static str = #name;
        }
    })
}
