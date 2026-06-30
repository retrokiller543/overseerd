//! Shared `provide = ..` codegen, used by `#[component]` and `#[service]`.
//!
//! For each trait a component provides, emits a `ProviderDescriptor` registered
//! into the `PROVIDERS` slice, plus an `erase` fn that re-types the *already
//! built* `Arc<Concrete>` as `Arc<dyn Trait + Send + Sync>`. The erase is a plain
//! `Arc::clone` + unsizing coercion — the same single instance, aliased under the
//! trait's key — never a second construction.

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::{Ident, LitStr};

use crate::extend::ParseKeyed;
use crate::{attr::ComponentArgs, paths::Paths};

/// Emits the provider registrations for `self_ident` given the parsed args.
/// Empty when the component provides nothing.
pub fn generate_providers<Ext: ParseKeyed>(
    self_ident: &Ident,
    args: &ComponentArgs<Ext>,
    paths: &Paths,
) -> TokenStream {
    if args.provide.is_empty() {
        return quote!();
    }

    let self_name = LitStr::new(&self_ident.to_string(), self_ident.span());
    let boxed_component = paths.core("BoxedComponent");
    let distributed_slice = paths.core("linkme::distributed_slice");
    let linkme_crate = paths.core("linkme");
    let provider_descriptor = paths.core("ProviderDescriptor");
    let providers_slice = paths.core("PROVIDERS");
    let type_descriptor = paths.core("TypeDescriptor");
    let live = paths.core("Live");

    // Qualifier defaults to the component's id (explicit `id`, else lowercased
    // type name), overridable with `qualifier = ".."`.
    let qualifier = args
        .qualifier
        .clone()
        .or_else(|| args.id.clone())
        .unwrap_or_else(|| LitStr::new(&self_ident.to_string().to_lowercase(), self_ident.span()));
    let primary = args.primary;

    let entries = args.provide.iter().enumerate().map(|(i, dyn_ty)| {
        // The trait object as written (`dyn Trait`). Both this provide side and
        // the dependency side spell it identically, so their keys match. The
        // trait must be `Send + Sync` (via supertraits, since components are
        // shared across threads); otherwise the erase fn's `Box<dyn Any + Send +
        // Sync>` storage fails to compile, pointing the author at the missing bound.
        let trait_name = LitStr::new(&dyn_ty.to_token_stream().to_string(), Span::call_site());
        let erase_ident = format_ident!("__overseerd_erase_{}", i);
        let provider_ident = format_ident!("__OVERSEERD_PROVIDER_{}", i);

        quote! {
            fn #erase_ident(__concrete: &#boxed_component) -> #boxed_component {
                let __arc = __concrete
                    .value
                    .downcast_ref::<#live<#self_ident>>()
                    .expect("provider concrete type mismatch")
                    .snapshot();
                let __erased: ::std::sync::Arc<#dyn_ty> = __arc;

                #boxed_component {
                    ty: #type_descriptor::of::<#dyn_ty>(#trait_name),
                    value: ::std::boxed::Box::new(#live::new(__erased)),
                }
            }

            #[#distributed_slice(#providers_slice)]
            #[linkme(crate = #linkme_crate)]
            static #provider_ident: #provider_descriptor = #provider_descriptor {
                trait_ty: #type_descriptor::of::<#dyn_ty>(#trait_name),
                concrete_ty: #type_descriptor::of::<#self_ident>(#self_name),
                qualifier: #qualifier,
                primary: #primary,
                erase: #erase_ident,
            };
        }
    });

    quote! { #(#entries)* }
}
