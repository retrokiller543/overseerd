//! Shared field-injection factory generation, used by `#[service]` (as the
//! overridable default) and `#[component]` (as the component's factory).
//!
//! Emits a singleton factory that constructs the struct field by field. Every
//! field is a **dependency** resolved from the container, unless it carries
//! `#[default]`, which makes it local owned state built via `Default::default()`.
//! A dependency field's type is its [`Injectable`] *handle*; the wrapper around
//! that handle selects the edge shape:
//! - `H` — a required single dependency (`Arc<T>`, or a by-value `Injectable`);
//! - `Option<H>` — an optional dependency (resolves to `None` if absent);
//! - `Dynamic<H>` — a runtime-provided dependency (exempt from static validation);
//! - `Option<Dynamic<H>>` — both.
//!
//! Each dependency is keyed under `<H as Injectable>::Target`, so a blanket
//! `Arc<T>` keys by `T` while a by-value handle keys by itself.
//!
//! [`Injectable`]: overseer_core::Injectable

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{Field, Fields, ItemStruct, LitStr, spanned::Spanned};

use crate::{attr, paths::overseer_path};

pub fn field_injection_component(
    item: &mut ItemStruct,
    id: &LitStr,
    name: &LitStr,
    default_factory: bool,
) -> TokenStream {
    let self_ident = item.ident.clone();
    let boxed_component = overseer_path("BoxedComponent");
    let component_construction_context = overseer_path("ComponentConstructionContext");
    let component_descriptor = overseer_path("ComponentDescriptor");
    let component_scope = overseer_path("ComponentScope");
    let components_slice = overseer_path("COMPONENTS");
    let dependency_descriptor = overseer_path("DependencyDescriptor");
    let distributed_slice = overseer_path("linkme::distributed_slice");
    let linkme_crate = overseer_path("linkme");
    let result = overseer_path("Result");
    let type_descriptor = overseer_path("TypeDescriptor");

    let mut inits = Vec::new();
    let mut dep_descriptors = Vec::new();

    let mut plan = |inits: &mut Vec<TokenStream>, prefix: TokenStream, field: &mut Field| {
        let FieldPlan { value, dependency } = plan_field(field);

        inits.push(quote!(#prefix #value));

        if let Some(dep) = dependency {
            dep_descriptors.push(dep);
        }
    };

    let construct = match &mut item.fields {
        Fields::Named(named) => {
            for field in &mut named.named {
                let field_ident = field.ident.clone().expect("named field");
                plan(&mut inits, quote!(#field_ident:), field);
            }

            quote!(#self_ident { #(#inits),* })
        }

        Fields::Unnamed(unnamed) => {
            for field in &mut unnamed.unnamed {
                plan(&mut inits, quote!(), field);
            }

            quote!(#self_ident( #(#inits),* ))
        }

        Fields::Unit => quote!(#self_ident),
    };

    let dependency_count = dep_descriptors.len();

    quote! {
        #[allow(unused_variables)]
        fn __overseer_factory(
            cx: &mut #component_construction_context,
        ) -> ::core::pin::Pin<
            ::std::boxed::Box<
                dyn ::core::future::Future<
                    Output = #result<#boxed_component>,
                > + ::core::marker::Send + '_,
            >,
        > {
            ::std::boxed::Box::pin(async move {
                let __instance = #construct;

                ::core::result::Result::Ok(#boxed_component {
                    ty: #type_descriptor::of::<#self_ident>(#name),
                    value: ::std::boxed::Box::new(::std::sync::Arc::new(__instance)),
                })
            })
        }

        static __OVERSEER_DEPS: [#dependency_descriptor; #dependency_count] = [
            #(#dep_descriptors),*
        ];

        #[#distributed_slice(#components_slice)]
        #[linkme(crate = #linkme_crate)]
        static __OVERSEER_COMPONENT: #component_descriptor =
            #component_descriptor {
                id: #id,
                name: #name,
                ty: #type_descriptor::of::<#self_ident>(#name),
                scope: #component_scope::Singleton,
                dependencies: &__OVERSEER_DEPS,
                factory: ::core::option::Option::Some(__overseer_factory),
                default_factory: #default_factory,
            };
    }
}

/// One field's contribution: the value expression that builds it in the struct
/// literal, and the dependency descriptor it registers (absent for `#[default]`
/// local state).
struct FieldPlan {
    value: TokenStream,
    dependency: Option<TokenStream>,
}

/// Classifies a field and, as a side effect, strips the `#[default]` marker so
/// the emitted struct stays valid.
fn plan_field(field: &mut Field) -> FieldPlan {
    let had_default = field.attrs.iter().any(|a| a.path().is_ident("default"));
    field.attrs.retain(|a| !a.path().is_ident("default"));

    if had_default {
        return FieldPlan {
            value: quote!(::core::default::Default::default()),
            dependency: None,
        };
    }

    let cardinality = overseer_path("Cardinality");
    let dynamic_ty = overseer_path("Dynamic");
    let error = overseer_path("Error");
    let injectable = overseer_path("Injectable");
    let type_descriptor = overseer_path("TypeDescriptor");
    let dependency_descriptor = overseer_path("DependencyDescriptor");

    // Peel the edge-shape wrappers off the field type: `Option<…>` marks the
    // edge optional, `Dynamic<…>` marks it runtime-provided; the remaining type
    // is the `Injectable` handle to resolve.
    let ty = &field.ty;
    let (optional, after_option) = match attr::option_inner(ty) {
        Some(inner) => (true, inner),
        None => (false, ty.clone()),
    };
    let (dynamic, handle) = match attr::dynamic_inner(&after_option) {
        Some(inner) => (true, inner),
        None => (false, after_option),
    };

    let dep_name_str = match attr::arc_inner_type(&handle) {
        Ok(inner) => inner.to_token_stream().to_string(),
        Err(_) => handle.to_token_stream().to_string(),
    };
    let dep_name = LitStr::new(&dep_name_str, handle.span());

    let resolved = quote!(cx.resolve::<#handle>());
    let value = match (optional, dynamic) {
        (false, false) => quote!(#resolved.ok_or(#error::MissingComponent(#dep_name))?),
        (false, true) => quote!(#dynamic_ty(#resolved.ok_or(#error::MissingComponent(#dep_name))?)),
        (true, false) => resolved,
        (true, true) => quote!(#resolved.map(#dynamic_ty)),
    };

    let dependency = quote! {
        #dependency_descriptor {
            name: #dep_name,
            ty: #type_descriptor::of::<<#handle as #injectable>::Target>(#dep_name),
            cardinality: #cardinality::One,
            optional: #optional,
            dynamic: #dynamic,
        }
    };

    FieldPlan {
        value,
        dependency: Some(dependency),
    }
}