//! `#[injectable]` expansion (trait): marks a trait as injectable as
//! `Arc<dyn Trait>`.
//!
//! Natively, the trait gains `RuntimeDescriptor<ComponentDescriptor>` so its
//! descriptor remains available through a trait object. Wasm retains the
//! original trait because the DI descriptor surface is native-only.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemTrait, Token, ext::IdentExt, parse::Parse};

use crate::di;
use crate::paths::Paths;

/// Path overrides accepted by `#[injectable]`.
#[derive(Default)]
pub struct InjectableArgs {
    overseerd: Option<syn::Path>,
    krate: Option<syn::Path>,
}

impl InjectableArgs {
    pub(crate) fn paths(&self, default: Paths) -> Paths {
        default.resolve(self.overseerd.clone(), self.krate.clone())
    }
}

impl Parse for InjectableArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut args = Self::default();

        while !input.is_empty() {
            let key = syn::Ident::parse_any(input)?;

            match key.to_string().as_str() {
                "overseerd" => args.overseerd = Some(crate::attr::parse_path_override(input)?),
                "crate" => args.krate = Some(crate::attr::parse_path_override(input)?),
                _ => {
                    return Err(syn::Error::new_spanned(
                        key,
                        "unknown argument; expected one of: `overseerd`, `crate`",
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

pub fn expand(item: ItemTrait, paths: &Paths) -> TokenStream {
    let mut native_item = item.clone();
    let runtime_descriptor = paths.core("RuntimeDescriptor");
    let component_descriptor = paths.core("ComponentDescriptor");
    let provide = di::injectable_impl(&item.ident, paths);

    if native_item.colon_token.is_none() {
        native_item.colon_token = Some(Default::default());
    }

    native_item
        .supertraits
        .push(syn::parse_quote!(#runtime_descriptor<#component_descriptor>));

    let native = crate::gate::native_only(quote! {
        #native_item

        #provide
    });

    quote! {
        #[cfg(target_family = "wasm")]
        #item

        #native
    }
}

#[cfg(test)]
mod tests;
