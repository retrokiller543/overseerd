//! `#[component]` expansion (struct).
//!
//! Declares a system-constructed singleton component: implements `Component`
//! and emits a field-injection factory registered with `inventory`, so the
//! container builds it from its dependencies (`Arc<T>` fields resolved, other
//! fields `Default`-constructed). Unlike `#[service]` there is no versioning or
//! RPC surface. For construction that field injection can't express, provide the
//! instance via `DaemonBuilder::with_component` instead.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemStruct, LitStr};

use crate::{attr::ServiceArgs, inject, paths::overseer_path};

pub fn expand(args: ServiceArgs, item: ItemStruct) -> syn::Result<TokenStream> {
    let self_ident = item.ident.clone();

    let id = args
        .id
        .unwrap_or_else(|| LitStr::new(&self_ident.to_string().to_lowercase(), self_ident.span()));
    let name = args
        .name
        .unwrap_or_else(|| LitStr::new(&self_ident.to_string(), self_ident.span()));

    let factory = inject::field_injection_component(&item, &id, &name, false);
    let component = overseer_path("Component");

    Ok(quote! {
        #item

        impl #component for #self_ident {
            const ID: &'static str = #id;
            const NAME: &'static str = #name;
        }

        const _: () = {
            #factory
        };
    })
}
