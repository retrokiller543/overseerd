//! `#[service]` expansion (struct).
//!
//! Declares a service's identity, tied to the struct's type, and submits it to
//! `inventory`. When every field is an `Arc<T>` dependency (or the struct is a
//! unit), it also emits a **default** field-injection singleton factory; an
//! `#[init]` in a `#[handlers]` impl overrides it. If the fields aren't all
//! injectable, no default factory is generated and construction must come from
//! `#[init]` or `DaemonBuilder::with_component`.

use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{Fields, ItemStruct, LitStr, spanned::Spanned};

use crate::attr::{self, ServiceArgs};

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

    let default_component = field_injection_component(&item, &self_name);

    let service_static =
        format_ident!("__OVERSEER_SERVICE_{}", self_ident.to_string().to_uppercase());

    Ok(quote! {
        #item

        impl ::overseer_core::Component for #self_ident {
            const ID: &'static str = #id;
            const NAME: &'static str = #name;
        }

        const _: () = {
            #default_component

            static #service_static: ::overseer_core::ServiceDescriptor =
                ::overseer_core::ServiceDescriptor {
                    id: #id,
                    name: #name,
                    ty: ::overseer_core::TypeDescriptor::of::<#self_ident>(#self_name),
                    version: #version,
                };

            ::overseer_core::inventory::submit! {
                ::overseer_core::Descriptor::Service(&#service_static)
            }
        };
    })
}

/// Emits a default singleton factory that builds the struct by resolving each
/// field from the container. Returns empty tokens (no default factory) if the
/// fields aren't all `Arc<T>` — then construction must be explicit.
fn field_injection_component(item: &ItemStruct, self_name: &LitStr) -> TokenStream {
    let self_ident = &item.ident;

    let mut field_inits = Vec::new();
    let mut dep_types = Vec::new();

    match &item.fields {
        Fields::Named(named) => {
            for field in &named.named {
                let Ok(inner) = attr::arc_inner_type(&field.ty) else {
                    return quote!();
                };
                let field_ident = field.ident.as_ref().expect("named field");
                let inner_name = LitStr::new(&inner.to_token_stream().to_string(), inner.span());

                field_inits.push(quote! {
                    #field_ident: cx
                        .resolve::<#inner>()
                        .ok_or(::overseer_core::Error::MissingComponent(#inner_name))?
                });
                dep_types.push(inner);
            }
        }

        Fields::Unit => {}

        Fields::Unnamed(_) => return quote!(),
    }

    let construct = match &item.fields {
        Fields::Named(_) => quote!(#self_ident { #(#field_inits),* }),
        Fields::Unit => quote!(#self_ident),
        Fields::Unnamed(_) => return quote!(),
    };

    let dependency_descriptors = dep_types.iter().map(|t| {
        let dep_name = LitStr::new(&t.to_token_stream().to_string(), t.span());

        quote! {
            ::overseer_core::DependencyDescriptor {
                name: #dep_name,
                ty: ::overseer_core::TypeDescriptor::of::<#t>(#dep_name),
                optional: false,
            }
        }
    });
    let dependency_count = dep_types.len();

    quote! {
        #[allow(unused_variables)]
        fn __overseer_default_factory(
            cx: &mut ::overseer_core::ComponentConstructionContext,
        ) -> ::core::pin::Pin<
            ::std::boxed::Box<
                dyn ::core::future::Future<
                    Output = ::overseer_core::Result<::overseer_core::BoxedComponent>,
                > + ::core::marker::Send + '_,
            >,
        > {
            ::std::boxed::Box::pin(async move {
                let __instance = #construct;

                ::core::result::Result::Ok(::overseer_core::BoxedComponent {
                    ty: ::overseer_core::TypeDescriptor::of::<#self_ident>(#self_name),
                    value: ::std::boxed::Box::new(::std::sync::Arc::new(__instance)),
                })
            })
        }

        static __OVERSEER_DEFAULT_DEPS: [::overseer_core::DependencyDescriptor; #dependency_count] = [
            #(#dependency_descriptors),*
        ];

        static __OVERSEER_DEFAULT_COMPONENT: ::overseer_core::ComponentDescriptor =
            ::overseer_core::ComponentDescriptor {
                id: #self_name,
                name: #self_name,
                ty: ::overseer_core::TypeDescriptor::of::<#self_ident>(#self_name),
                scope: ::overseer_core::ComponentScope::Singleton,
                dependencies: &__OVERSEER_DEFAULT_DEPS,
                factory: ::core::option::Option::Some(__overseer_default_factory),
                default_factory: true,
            };

        ::overseer_core::inventory::submit! {
            ::overseer_core::Descriptor::Component(&__OVERSEER_DEFAULT_COMPONENT)
        }
    }
}
