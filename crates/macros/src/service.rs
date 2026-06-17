//! `#[service]` expansion (struct).
//!
//! Declares a service's identity, tied to the struct's type, and submits it to
//! `inventory`. Implements `Component` + `ServiceComponent` (carrying the
//! version on the type). When every field is an `Arc<T>` dependency (or the
//! struct is a unit), it also emits a **default** field-injection singleton
//! factory; an `#[init]` in a `#[handlers]` impl overrides it. If the fields
//! aren't all injectable, no default factory is generated and construction must
//! come from `#[init]` or `DaemonBuilder::with_component`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{ItemStruct, LitStr};

use crate::{attr::ServiceArgs, inject, paths::overseer_path};

pub fn expand(args: ServiceArgs, item: ItemStruct) -> syn::Result<TokenStream> {
    let self_ident = item.ident.clone();
    let self_name = LitStr::new(&self_ident.to_string(), self_ident.span());

    let id = args
        .id
        .unwrap_or_else(|| LitStr::new(&self_ident.to_string().to_lowercase(), self_ident.span()));
    let name = args
        .name
        .unwrap_or_else(|| LitStr::new(&self_ident.to_string(), self_ident.span()));
    let version = match &args.version {
        Some(v) => quote!(::core::option::Option::Some(#v)),
        None => quote!(::core::option::Option::None),
    };

    let default_component = inject::field_injection_component(&item, &id, &name, true);

    let service_static = format_ident!(
        "__OVERSEER_SERVICE_{}",
        self_ident.to_string().to_uppercase()
    );
    let component = overseer_path("Component");
    let descriptor = overseer_path("Descriptor");
    let inventory_submit = overseer_path("inventory::submit");
    let service_component = overseer_path("ServiceComponent");
    let service_descriptor = overseer_path("ServiceDescriptor");
    let type_descriptor = overseer_path("TypeDescriptor");

    Ok(quote! {
        #item

        impl #component for #self_ident {
            const ID: &'static str = #id;
            const NAME: &'static str = #name;
        }

        impl #service_component for #self_ident {
            const VERSION: ::core::option::Option<&'static str> = #version;
        }

        const _: () = {
            #default_component

            static #service_static: #service_descriptor =
                #service_descriptor {
                    id: #id,
                    name: #name,
                    ty: #type_descriptor::of::<#self_ident>(#self_name),
                    version: #version,
                };

            #inventory_submit! {
                #descriptor::Service(&#service_static)
            }
        };
    })
}
