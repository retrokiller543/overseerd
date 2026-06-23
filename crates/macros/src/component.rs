//! `#[component]` expansion (struct).
//!
//! Declares a system-constructed singleton component: implements `Component`
//! and emits a field-injection factory registered into the `COMPONENTS` slice,
//! so the container builds it from its dependencies (each field injected unless
//! marked `#[default]`, which builds it via `Default`). Unlike `#[service]` there
//! is no versioning or RPC surface. For construction that field injection can't
//! express, provide the instance via `DaemonBuilder::with_component` instead.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemStruct, LitStr};

use crate::{attr::ServiceArgs, di, handle, inject, paths::overseerd_path, provide};

pub fn expand(args: ServiceArgs, mut item: ItemStruct) -> syn::Result<TokenStream> {
    let self_ident = item.ident.clone();
    let providers = provide::generate_providers(&self_ident, &args);
    let handle = handle::handle_impl(&self_ident, args.by_value);
    let handle_items = &handle.items;
    let injectable = &handle.injectable;
    let provide_impl = di::provide_impl(&self_ident);

    let id = args
        .id
        .unwrap_or_else(|| LitStr::new(&self_ident.to_string().to_lowercase(), self_ident.span()));
    let name = args
        .name
        .unwrap_or_else(|| LitStr::new(&self_ident.to_string(), self_ident.span()));

    let scope_variant = args
        .scope
        .clone()
        .unwrap_or_else(|| syn::Ident::new("Singleton", self_ident.span()));
    let factory = inject::field_injection_component(&mut item, &id, &name, false, &scope_variant);
    let component = overseerd_path("Component");

    Ok(quote! {
        #item

        impl #component for #self_ident {
            const ID: &'static str = #id;
            const NAME: &'static str = #name;
            #handle_items
        }

        #injectable

        #provide_impl

        const _: () = {
            #factory

            #providers
        };
    })
}
