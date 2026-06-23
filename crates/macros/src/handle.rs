//! Shared codegen for a component's storage handle (`Component::Handle` +
//! `into_handle`), used by `#[derive(Component)]`, `#[component]`, and
//! `#[service]`.
//!
//! The default wraps the component in `Arc<Self>` (auto-wrapped, the common
//! case). `#[component(by_value)]` instead stores it as `Self` and emits the
//! self-`Injectable` impl that makes `Self` a valid handle — for types that
//! manage their own sharing (typically internally `Arc`, cheap to clone).

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

use crate::paths::overseerd_path;

/// The pieces a `Component` impl needs for its handle: the associated-type and
/// `into_handle` body to splice into the impl, plus an optional standalone
/// `Injectable` impl (non-empty only for `by_value`).
pub struct HandleImpl {
    pub items: TokenStream,
    pub injectable: TokenStream,
}

pub fn handle_impl(self_ident: &Ident, by_value: bool) -> HandleImpl {
    if by_value {
        let injectable = overseerd_path("Injectable");

        HandleImpl {
            items: quote! {
                type Handle = Self;

                fn into_handle(self) -> Self::Handle {
                    self
                }
            },
            injectable: quote! {
                impl #injectable for #self_ident {
                    type Target = #self_ident;
                }
            },
        }
    } else {
        HandleImpl {
            items: quote! {
                type Handle = ::std::sync::Arc<Self>;

                fn into_handle(self) -> Self::Handle {
                    ::std::sync::Arc::new(self)
                }
            },
            injectable: quote!(),
        }
    }
}
