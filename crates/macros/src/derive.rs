//! `#[derive(Component)]` — implements the `Component` metadata trait for plain
//! dependency types (e.g. config, pools) so they can be registered via
//! `DaemonBuilder::with_component`.
//!
//! `ID` defaults to the lowercased type name and `NAME` to the type name; both
//! can be overridden with `#[component(id = "...", name = "...")]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, LitStr};

use crate::{attr::ServiceArgs, di, handle, paths::overseerd_path};

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let ident = &input.ident;

    let mut overrides = ServiceArgs::default();

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
    let component = overseerd_path("Component");
    let handle = handle::handle_impl(ident, overrides.by_value);
    let handle_items = &handle.items;
    let injectable = &handle.injectable;
    let provide = di::provide_impl(ident);
    let wired = di::wired_impl(ident, &[]);

    Ok(quote! {
        impl #component for #ident {
            const ID: &'static str = #id;
            const NAME: &'static str = #name;
            #handle_items
        }

        #injectable

        #provide

        #wired
    })
}
