//! `#[service]` expansion (struct).
//!
//! Declares a service's identity, tied to the struct's type, and registers it
//! into the `SERVICES` slice. Implements `Component` + `ServiceComponent`
//! (carrying the version on the type). It also declares the service's own
//! `{Service}Rpcs` distributed slice — the slice each `#[handlers]` block appends
//! its `RpcGroup` to — and an `impl ServiceRpcs` returning it, then points the
//! `ServiceDescriptor.rpcs` fn pointer at that impl so the service owns its RPC
//! surface. It also emits a **default** field-injection
//! singleton factory — each field is injected unless marked `#[default]` (local
//! state built via `Default`) — overridable by an `#[init]` in a `#[handlers]`
//! impl. A field whose type is not an `Injectable` handle and is not `#[default]`
//! fails to compile; construct such a type via `#[init]` or
//! `DaemonBuilder::with_component`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{ItemStruct, LitStr};

use crate::{attr::ServiceArgs, di, handle, inject, paths::overseerd_path, provide};

pub fn expand(args: ServiceArgs, mut item: ItemStruct) -> syn::Result<TokenStream> {
    let self_ident = item.ident.clone();
    let self_name = LitStr::new(&self_ident.to_string(), self_ident.span());
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
    let version = match &args.version {
        Some(v) => quote!(::core::option::Option::Some(#v)),
        None => quote!(::core::option::Option::None),
    };

    if let Some(scope) = &args.scope
        && scope != "Singleton"
    {
        return Err(syn::Error::new(
            scope.span(),
            "#[service] components are always singletons; `scope` is only valid on #[component]",
        ));
    }

    let singleton = syn::Ident::new("Singleton", self_ident.span());
    let default_component =
        inject::field_injection_component(&mut item, &id, &name, true, &singleton);

    let service_static = format_ident!(
        "__OVERSEERD_SERVICE_{}",
        self_ident.to_string().to_uppercase()
    );
    let rpcs_slice = args
        .rpc_slice
        .clone()
        .unwrap_or_else(|| format_ident!("{}Rpcs", self_ident));
    let component = overseerd_path("Component");
    let descriptor_trait = overseerd_path("Descriptor");
    let distributed_slice = overseerd_path("linkme::distributed_slice");
    let linkme_crate = overseerd_path("linkme");
    let rpc_group = overseerd_path("RpcGroup");
    let service_component = overseerd_path("ServiceComponent");
    let service_descriptor = overseerd_path("ServiceDescriptor");
    let service_rpcs = overseerd_path("ServiceRpcs");
    let services_slice = overseerd_path("SERVICES");
    let type_descriptor = overseerd_path("TypeDescriptor");

    Ok(quote! {
        #item

        impl #component for #self_ident {
            const ID: &'static str = #id;
            const NAME: &'static str = #name;
            #handle_items
        }

        #injectable

        #provide_impl

        impl #service_component for #self_ident {
            const VERSION: ::core::option::Option<&'static str> = #version;
        }

        #[#distributed_slice]
        #[linkme(crate = #linkme_crate)]
        #[allow(non_upper_case_globals)]
        pub static #rpcs_slice: [#rpc_group];

        impl #service_rpcs for #self_ident {
            fn rpc_groups() -> &'static [#rpc_group] {
                &#rpcs_slice
            }
        }

        const _: () = {
            #default_component

            const __OVERSEERD_SERVICE_DESCRIPTOR: #service_descriptor =
                #service_descriptor {
                    id: #id,
                    name: #name,
                    ty: #type_descriptor::of::<#self_ident>(#self_name),
                    version: #version,
                    rpcs: <#self_ident as #service_rpcs>::rpc_groups,
                };

            impl #descriptor_trait<#service_descriptor> for #self_ident {
                const DESCRIPTOR: #service_descriptor = __OVERSEERD_SERVICE_DESCRIPTOR;
            }

            #[#distributed_slice(#services_slice)]
            #[linkme(crate = #linkme_crate)]
            static #service_static: #service_descriptor = __OVERSEERD_SERVICE_DESCRIPTOR;

            #providers
        };
    })
}
