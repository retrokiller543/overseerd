//! `#[derive(ConfigProperties)]` — implements the `ConfigProperties` trait for a
//! config struct and, when given `#[config(path = "..")]`, auto-registers a binding
//! into the `CONFIG_BINDINGS` slice so `auto_discover` picks it up.
//!
//! `NAME` defaults to the type name (override with `#[config(name = "..")]`). The
//! type must also be `Deserialize`. Omitting `path` leaves binding to an explicit
//! `DaemonBuilder::config::<T>(path)` call — needed when the same type binds at
//! several paths.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    DeriveInput, Ident, LitStr, Token,
    parse::{Parse, ParseStream},
};

use crate::paths::overseerd_path;

/// Arguments of the `#[config(...)]` helper attribute on a config struct.
#[derive(Default)]
struct ConfigArgs {
    name: Option<LitStr>,
    path: Option<LitStr>,
}

impl Parse for ConfigArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = ConfigArgs::default();

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match key.to_string().as_str() {
                "name" => args.name = Some(input.parse()?),
                "path" => args.path = Some(input.parse()?),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown argument `{other}`, expected `name` or `path`"),
                    ));
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(args)
    }
}

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let ident = &input.ident;

    let mut args = ConfigArgs::default();

    for attr in &input.attrs {
        if attr.path().is_ident("config") {
            args = attr.parse_args::<ConfigArgs>()?;
        }
    }

    let name = args
        .name
        .unwrap_or_else(|| LitStr::new(&ident.to_string(), ident.span()));
    let config_properties = overseerd_path("ConfigProperties");
    let config_binding_descriptor = overseerd_path("ConfigBindingDescriptor");
    let config_bindings = overseerd_path("CONFIG_BINDINGS");
    let distributed_slice = overseerd_path("linkme::distributed_slice");
    let linkme_crate = overseerd_path("linkme");
    let type_descriptor = overseerd_path("TypeDescriptor");

    // A baked-in path auto-registers the binding; without one the binding is made
    // explicitly at the builder (the multi-path case).
    let registration = match args.path {
        Some(path) => quote! {
            const _: () = {
                #[#distributed_slice(#config_bindings)]
                #[linkme(crate = #linkme_crate)]
                static __OVERSEERD_CONFIG_BINDING: #config_binding_descriptor =
                    #config_binding_descriptor {
                        ty: #type_descriptor::of::<#ident>(#name),
                        path: #path,
                        bind: <#ident as #config_properties>::bind,
                    };
            };
        },

        None => quote!(),
    };

    Ok(quote! {
        impl #config_properties for #ident {
            const NAME: &'static str = #name;
        }

        #registration
    })
}
