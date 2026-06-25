//! `#[config]` — implements the `ConfigProperties` trait for a config type and,
//! when given `#[config(path = "..")]`, auto-registers a binding into the
//! `CONFIG_BINDINGS` slice so `auto_discover` picks it up.
//!
//! `NAME` defaults to the type name (override with `#[config(name = "..")]`). The
//! type must also be `Deserialize`. Omitting `path` leaves binding to an explicit
//! `DaemonBuilder::config::<T>(path)` call — needed when the same type binds at
//! several paths.
//!
//! Applies to a `struct` or an `enum` (serde handles either). Fields may carry a
//! field-level `#[default = ".."]` whose value is a template string merged under the
//! config before deserialization, so a missing field falls back to a (possibly
//! templated) default that resolves through the normal `${..}` pipeline.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, Fields, Ident, Lit, LitStr, Meta, Token,
    parse::{Parse, ParseStream},
};

use crate::paths::overseerd_path;

/// Arguments of the `#[config(...)]` attribute on a config type.
#[derive(Default)]
pub struct ConfigArgs {
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

pub fn expand(args: ConfigArgs, mut item: DeriveInput) -> syn::Result<TokenStream> {
    let ident = item.ident.clone();

    let name = args
        .name
        .unwrap_or_else(|| LitStr::new(&ident.to_string(), ident.span()));
    let config_properties = overseerd_path("ConfigProperties");
    let config_binding_descriptor = overseerd_path("ConfigBindingDescriptor");
    let config_bindings = overseerd_path("CONFIG_BINDINGS");
    let distributed_slice = overseerd_path("linkme::distributed_slice");
    let linkme_crate = overseerd_path("linkme");
    let type_descriptor = overseerd_path("TypeDescriptor");

    // Collect (and strip) the field-level `#[default = ".."]` attributes, then build the
    // `defaults()` method. Stripping is required so the sibling `#[derive(Deserialize)]`
    // does not see the unknown attribute.
    let defaults_method = build_defaults(&mut item)?;

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
        #item

        impl #config_properties for #ident {
            const NAME: &'static str = #name;

            #defaults_method
        }

        #registration
    })
}

/// Builds the `defaults()` method body from the type's field-level `#[default = ".."]`
/// attributes, stripping each consumed attribute from `item`.
///
/// Returns an empty token stream (use the trait default — no defaults) when no field
/// carries one. A struct yields `DefaultSpec::Fields`; an enum yields
/// `DefaultSpec::Variants`, including only variants that have at least one defaulted field.
fn build_defaults(item: &mut DeriveInput) -> syn::Result<TokenStream> {
    let default_spec = overseerd_path("DefaultSpec");

    match &mut item.data {
        Data::Struct(data) => {
            let fields = take_field_defaults(&mut data.fields)?;

            if fields.is_empty() {
                return Ok(quote!());
            }

            let entries = fields.iter().map(|(field, lit)| {
                quote! { (::std::string::String::from(#field), ::std::string::String::from(#lit)) }
            });

            Ok(quote! {
                fn defaults() -> #default_spec {
                    #default_spec::Fields(::std::vec![ #(#entries),* ])
                }
            })
        }

        Data::Enum(data) => {
            let mut variants = Vec::new();

            for variant in data.variants.iter_mut() {
                let variant_name = variant.ident.to_string();
                let fields = take_field_defaults(&mut variant.fields)?;

                if !fields.is_empty() {
                    variants.push((variant_name, fields));
                }
            }

            if variants.is_empty() {
                return Ok(quote!());
            }

            let entries = variants.iter().map(|(variant, fields)| {
                let field_entries = fields.iter().map(|(field, lit)| {
                    quote! { (::std::string::String::from(#field), ::std::string::String::from(#lit)) }
                });

                quote! {
                    (
                        ::std::string::String::from(#variant),
                        ::std::vec![ #(#field_entries),* ],
                    )
                }
            });

            Ok(quote! {
                fn defaults() -> #default_spec {
                    #default_spec::Variants(::std::vec![ #(#entries),* ])
                }
            })
        }

        Data::Union(data) => Err(syn::Error::new_spanned(
            data.union_token,
            "`#[config]` cannot be applied to a union",
        )),
    }
}

/// Pulls every `#[default = ".."]` off the named fields of `fields`, returning
/// `(field name, template literal)` pairs and removing the consumed attributes.
///
/// A default on an unnamed (tuple) field is rejected: there is no field name to key the
/// merged value by.
fn take_field_defaults(fields: &mut Fields) -> syn::Result<Vec<(String, LitStr)>> {
    let mut defaults = Vec::new();

    match fields {
        Fields::Named(named) => {
            for field in named.named.iter_mut() {
                if let Some(lit) = take_default_attr(&mut field.attrs)? {
                    let key = field
                        .ident
                        .as_ref()
                        .expect("named field has an identifier")
                        .to_string();

                    defaults.push((key, lit));
                }
            }
        }

        Fields::Unnamed(unnamed) => {
            for field in unnamed.unnamed.iter_mut() {
                if take_default_attr(&mut field.attrs)?.is_some() {
                    return Err(syn::Error::new_spanned(
                        field,
                        "`#[default = \"..\"]` is only supported on named fields",
                    ));
                }
            }
        }

        Fields::Unit => {}
    }

    Ok(defaults)
}

/// Removes a single `#[default = ".."]` attribute from `attrs`, returning its string
/// literal. Errors when the attribute is present but not in the `= "literal"` form.
fn take_default_attr(attrs: &mut Vec<Attribute>) -> syn::Result<Option<LitStr>> {
    let position = attrs
        .iter()
        .position(|attr| attr.path().is_ident("default"));

    let index = match position {
        Some(index) => index,
        None => return Ok(None),
    };

    let attr = attrs.remove(index);

    match &attr.meta {
        Meta::NameValue(nv) => match &nv.value {
            Expr::Lit(ExprLit {
                lit: Lit::Str(lit), ..
            }) => Ok(Some(lit.clone())),

            other => Err(syn::Error::new_spanned(
                other,
                "`#[default = ..]` expects a string literal template, e.g. `#[default = \"${tcp.ip}:8080\"]`",
            )),
        },

        other => Err(syn::Error::new_spanned(
            other,
            "`#[config]` field default must be written `#[default = \"..\"]`",
        )),
    }
}
