//! `#[service]` expansion: collect `#[rpc]` methods of an impl block into a
//! `ServiceDescriptor` plus per-method erased handler wrappers.
//!
//! Generated code is namespaced inside a `const _: () = { ... }` block so the
//! mangled wrapper fns and statics never leak into the user's namespace. The
//! `inventory::submit!` inside it registers the service for `auto_discover`.

use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{FnArg, ImplItem, ItemImpl, LitStr, Meta, Type, spanned::Spanned};

use crate::attr::{self, ServiceArgs};

pub fn expand(args: ServiceArgs, mut item: ItemImpl) -> syn::Result<TokenStream> {
    let self_ty = (*item.self_ty).clone();
    let self_ident = self_ty_ident(&self_ty)?;

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

    let mut wrappers = Vec::new();
    let mut descriptors = Vec::new();

    for impl_item in &mut item.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };

        let Some(pos) = method.attrs.iter().position(|a| a.path().is_ident("rpc")) else {
            continue;
        };

        let rpc_attr = method.attrs.remove(pos);
        let rpc_args = match &rpc_attr.meta {
            Meta::Path(_) => attr::RpcArgs { operation: None },
            _ => rpc_attr.parse_args::<attr::RpcArgs>()?,
        };

        let (wrapper, descriptor) = expand_method(&self_ty, &self_ident, method, rpc_args)?;

        wrappers.push(wrapper);
        descriptors.push(descriptor);
    }

    if descriptors.is_empty() {
        return Err(syn::Error::new_spanned(
            &item,
            "#[service] impl block has no #[rpc] methods",
        ));
    }

    let count = descriptors.len();
    let rpcs_static = format_ident!("__OVERSEER_RPCS_{}", self_ident.to_string().to_uppercase());
    let service_static =
        format_ident!("__OVERSEER_SERVICE_{}", self_ident.to_string().to_uppercase());
    let self_name = LitStr::new(&self_ident.to_string(), self_ident.span());

    Ok(quote! {
        #item

        const _: () = {
            #(#wrappers)*

            static #rpcs_static: [::overseer_core::RpcDescriptor; #count] = [
                #(#descriptors),*
            ];

            static #service_static: ::overseer_core::ServiceDescriptor =
                ::overseer_core::ServiceDescriptor {
                    id: #id,
                    name: #name,
                    ty: ::overseer_core::TypeDescriptor::of::<#self_ty>(#self_name),
                    version: #version,
                    rpcs: &#rpcs_static,
                };

            ::overseer_core::inventory::submit! {
                ::overseer_core::Descriptor::Service(&#service_static)
            }
        };
    })
}

/// Builds the erased handler wrapper and the `RpcDescriptor` for one method.
fn expand_method(
    self_ty: &Type,
    self_ident: &syn::Ident,
    method: &syn::ImplItemFn,
    rpc_args: attr::RpcArgs,
) -> syn::Result<(TokenStream, TokenStream)> {
    if method.sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            &method.sig,
            "rpc methods must be `async`",
        ));
    }

    if let Some(FnArg::Receiver(receiver)) = method.sig.inputs.first() {
        return Err(syn::Error::new_spanned(
            receiver,
            "rpc methods cannot take `self` yet — handlers are stateless; reach \
             connection-scoped state through extractors (`Conn`, `Extension<T>`)",
        ));
    }

    let method_ident = &method.sig.ident;
    let method_name = LitStr::new(&method_ident.to_string(), method_ident.span());
    let operation = attr::operation_variant(&rpc_args.operation)?;
    let output_ty = attr::result_ok_type(&method.sig.output)?;
    let output_name = LitStr::new(&output_ty.to_token_stream().to_string(), output_ty.span());

    let wrapper_ident = format_ident!(
        "__overseer_rpc_{}_{}",
        self_ident.to_string().to_lowercase(),
        method_ident
    );

    let wrapper = quote! {
        fn #wrapper_ident(
            ctx: ::overseer_core::RpcCallContext,
        ) -> ::core::pin::Pin<
            ::std::boxed::Box<
                dyn ::core::future::Future<
                    Output = ::overseer_core::Result<::overseer_core::RpcResponse>,
                > + ::core::marker::Send,
            >,
        > {
            ::overseer_core::dispatch_with(<#self_ty>::#method_ident, ctx)
        }
    };

    let descriptor = quote! {
        ::overseer_core::RpcDescriptor {
            name: #method_name,
            operation: ::overseer_core::OperationKind::#operation,
            parameters: &[],
            output: ::overseer_core::TypeDescriptor::of::<#output_ty>(#output_name),
            handler: #wrapper_ident,
        }
    };

    Ok((wrapper, descriptor))
}

/// Extracts the named type ident from an impl's `Self` type.
fn self_ty_ident(ty: &Type) -> syn::Result<syn::Ident> {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.clone())
            .ok_or_else(|| syn::Error::new_spanned(ty, "expected a named type")),
        _ => Err(syn::Error::new_spanned(
            ty,
            "#[service] must be applied to an impl of a named type",
        )),
    }
}
