//! Shared field-injection factory generation, used by `#[service]` (as the
//! overridable default) and `#[component]` (as the component's factory).
//!
//! Emits a singleton factory that constructs the struct field by field:
//! - an `Arc<T>` field is a dependency, resolved from the container;
//! - any other field is owned by the component and built via `Default::default()`
//!   (so its type must implement `Default`, or the component must be constructed
//!   another way — an `#[init]` constructor or `with_component`).

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{Fields, ItemStruct, LitStr, spanned::Spanned};

use crate::attr;

pub fn field_injection_component(
    item: &ItemStruct,
    id: &LitStr,
    name: &LitStr,
    default_factory: bool,
) -> TokenStream {
    let self_ident = &item.ident;

    let mut inits = Vec::new();
    let mut dep_types = Vec::new();

    let push_field = |inits: &mut Vec<TokenStream>,
                      dep_types: &mut Vec<syn::Type>,
                      prefix: TokenStream,
                      ty: &syn::Type| {
        match attr::arc_inner_type(ty) {
            Ok(inner) => {
                let inner_name = LitStr::new(&inner.to_token_stream().to_string(), inner.span());

                inits.push(quote! {
                    #prefix cx
                        .resolve::<#inner>()
                        .ok_or(::overseer_core::Error::MissingComponent(#inner_name))?
                });
                dep_types.push(inner);
            }

            Err(_) => {
                inits.push(quote!(#prefix ::core::default::Default::default()));
            }
        }
    };

    let construct = match &item.fields {
        Fields::Named(named) => {
            for field in &named.named {
                let field_ident = field.ident.as_ref().expect("named field");
                push_field(&mut inits, &mut dep_types, quote!(#field_ident:), &field.ty);
            }

            quote!(#self_ident { #(#inits),* })
        }

        Fields::Unnamed(unnamed) => {
            for field in &unnamed.unnamed {
                push_field(&mut inits, &mut dep_types, quote!(), &field.ty);
            }

            quote!(#self_ident( #(#inits),* ))
        }

        Fields::Unit => quote!(#self_ident),
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
        fn __overseer_factory(
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
                    ty: ::overseer_core::TypeDescriptor::of::<#self_ident>(#name),
                    value: ::std::boxed::Box::new(::std::sync::Arc::new(__instance)),
                })
            })
        }

        static __OVERSEER_DEPS: [::overseer_core::DependencyDescriptor; #dependency_count] = [
            #(#dependency_descriptors),*
        ];

        static __OVERSEER_COMPONENT: ::overseer_core::ComponentDescriptor =
            ::overseer_core::ComponentDescriptor {
                id: #id,
                name: #name,
                ty: ::overseer_core::TypeDescriptor::of::<#self_ident>(#name),
                scope: ::overseer_core::ComponentScope::Singleton,
                dependencies: &__OVERSEER_DEPS,
                factory: ::core::option::Option::Some(__overseer_factory),
                default_factory: #default_factory,
            };

        ::overseer_core::inventory::submit! {
            ::overseer_core::Descriptor::Component(&__OVERSEER_COMPONENT)
        }
    }
}