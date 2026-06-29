//! The base component macro: `ComponentArgs<Ext>` expansion.
//!
//! Declares a system-constructed singleton component — implements `Component`, emits a
//! field-injection factory registered into the `COMPONENTS` slice, and wires providers/handle.
//! `#[component]` is `ComponentArgs<NoExt>`; `#[service]` is `ComponentArgs<Router>` — the same
//! base skeleton plus the RPC service surface the extension appends.
//!
//! The base resolves the component identity (`id`/`name`/`scope`), hands it to the extension
//! via [`ParseItem<ComponentContext>`] (so the extension's output agrees with the component),
//! and defers its eager field-DI assertion when the extension reports the factory may be
//! overridden ([`ComponentExt::defers_factory`] — as a service is, by a `#[handlers]` `#[init]`).

use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemStruct, LitStr};

use crate::attr::ComponentArgs;
use crate::extend::{ComponentContext, ComponentExt};
use crate::{di, handle, inject, paths::overseerd_path, provide};

pub fn expand<Ext: ComponentExt>(
    mut args: ComponentArgs<Ext>,
    mut item: ItemStruct,
) -> syn::Result<TokenStream> {
    let self_ident = item.ident.clone();
    let providers = provide::generate_providers(&self_ident, &args);
    let handle = handle::handle_impl(&self_ident, args.by_value);
    let handle_items = &handle.items;
    let injectable = &handle.injectable;
    let provide_impl = di::provide_impl(&self_ident);

    let id = args
        .id
        .clone()
        .unwrap_or_else(|| LitStr::new(&self_ident.to_string().to_lowercase(), self_ident.span()));
    let name = args
        .name
        .clone()
        .unwrap_or_else(|| LitStr::new(&self_ident.to_string(), self_ident.span()));
    let type_name = LitStr::new(&self_ident.to_string(), self_ident.span());

    let scope_variant = args
        .scope
        .clone()
        .unwrap_or_else(|| syn::Ident::new("Singleton", self_ident.span()));
    let factories_slice = args
        .factory_slice
        .clone()
        .unwrap_or_else(|| inject::factories_slice_ident(&self_ident));
    let factories_infra = inject::factories_infrastructure(&self_ident, &factories_slice);
    let hooks_slice = inject::hooks_slice_ident(&self_ident);
    let hooks_infra = inject::hooks_infrastructure(&self_ident, &hooks_slice);

    // Hand the resolved identity to the extension before it emits, so its output (service
    // descriptor, route table, …) names the component consistently.
    let context = ComponentContext {
        ident: self_ident.clone(),
        type_name,
        id: id.clone(),
        name: name.clone(),
        scope: args.scope.clone(),
    };
    args.ext.parse_item(&context)?;

    // An explicit `factory = path` replaces the field-injection default; so does
    // `default_factory = false` (the manual case). Otherwise the default is emitted. The
    // eager DI assertion is deferred when the extension allows a later factory override.
    let explicit = args
        .factory
        .as_ref()
        .map(|path| inject::explicit_factory(path, &factories_slice));
    let emit_default = explicit.is_none() && !args.no_default_factory;
    let factory = inject::field_injection_component(
        &mut item,
        &id,
        &name,
        args.ext.defers_factory(),
        &scope_variant,
        &factories_slice,
        emit_default,
    );
    let component = overseerd_path("Component");

    // The extension's appended surface (nothing for `#[component]`; the service header, RPC
    // slice, and client for `#[service]`).
    let ext = &args.ext;

    Ok(quote! {
        #item

        impl #component for #self_ident {
            const ID: &'static str = #id;
            const NAME: &'static str = #name;
            #handle_items
        }

        #injectable

        #provide_impl

        #factories_infra

        #hooks_infra

        const _: () = {
            #factory

            #explicit

            #providers
        };

        #ext
    })
}
