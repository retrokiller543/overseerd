//! Shared codegen for `#[hook(Kind)]` methods, used by `#[methods]` and `#[handlers]`.
//!
//! A hook is an `async` method — optionally taking `&self` — whose parameters are the
//! kind's inputs (each a `HookParam<Kind>`, e.g. `CfgNext<T>` for `ConfigReload`), targeted
//! by an optional per-parameter `#[config("path")]`. Each hook is compiled to a deps
//! reporter and an erased call, registered into the type's `{Type}Hooks` slice. Component
//! dependencies are NOT parameters — a hook reaches them through `&self`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ImplItemFn, LitStr, Meta, Path, Type};

use crate::attr;
use crate::paths::Paths;

/// One hook parameter: its (kind-input) type and optional `#[config("path")]`.
struct HookParamInfo {
    ty: Type,
    path: Option<LitStr>,
}

/// The parsed shape of a `#[hook(Kind)]` method.
pub struct HookInfo {
    ident: syn::Ident,
    kind: Path,
    takes_self: bool,
    params: Vec<HookParamInfo>,
    is_result: bool,
}

/// Parses a `#[hook(kind)]` method, stripping per-parameter `#[config("..")]` attributes so
/// the re-emitted method stays valid. `kind` is the path from the `#[hook(..)]` attribute,
/// already removed by the caller.
pub fn parse_hook(method: &mut ImplItemFn, kind: Path) -> syn::Result<HookInfo> {
    if method.sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            &method.sig,
            "#[hook] methods must be async",
        ));
    }

    let mut takes_self = false;
    let mut params = Vec::new();

    for arg in &mut method.sig.inputs {
        match arg {
            FnArg::Receiver(receiver) => {
                if receiver.reference.is_none() || receiver.mutability.is_some() {
                    return Err(syn::Error::new_spanned(
                        receiver,
                        "a #[hook] receiver must be `&self` (or omit `self` entirely)",
                    ));
                }

                takes_self = true;
            }

            FnArg::Typed(typed) => {
                let path = take_config_path(&mut typed.attrs)?;

                params.push(HookParamInfo {
                    ty: (*typed.ty).clone(),
                    path,
                });
            }
        }
    }

    Ok(HookInfo {
        ident: method.sig.ident.clone(),
        kind,
        takes_self,
        params,
        is_result: attr::result_type_args(&method.sig.output).is_some(),
    })
}

/// Removes a `#[config("path")]` attribute from a parameter, returning its path literal.
fn take_config_path(attrs: &mut Vec<syn::Attribute>) -> syn::Result<Option<LitStr>> {
    let Some(pos) = attrs.iter().position(|a| a.path().is_ident("config")) else {
        return Ok(None);
    };

    let attr = attrs.remove(pos);

    let Meta::List(list) = &attr.meta else {
        return Err(syn::Error::new_spanned(
            &attr,
            "expected #[config(\"path\")]",
        ));
    };

    let path = syn::parse2::<LitStr>(list.tokens.clone())?;

    Ok(Some(path))
}

/// Emits a hook's deps reporter, erased call, and `HookDescriptor` (appended to the type's
/// `{Type}Hooks` slice). `index` disambiguates multiple hooks on one type.
#[allow(clippy::too_many_arguments)]
pub fn generate_hook(
    self_ty: &Type,
    name: &LitStr,
    hooks_slice: &syn::Ident,
    info: &HookInfo,
    index: usize,
    paths: &Paths,
) -> TokenStream {
    let any = quote!(::core::any);
    let hook_kind = paths.core("HookKind");
    let hook_param = paths.core("HookParam");
    let hook_descriptor = paths.core("HookDescriptor");
    let resolver_ctx = paths.core("ResolverCtx");
    let resolver_ctx_ext = paths.core("ResolverCtxExt");
    let component_source = paths.core("ComponentSource");
    let component = paths.core("Component");
    let dependency_descriptor = paths.core("DependencyDescriptor");
    let type_descriptor = paths.core("TypeDescriptor");
    let hook_error = paths.core("HookError");
    let hook_result = paths.core("HookResult");
    let distributed_slice = paths.core("linkme::distributed_slice");
    let linkme_crate = paths.core("linkme");

    let kind = &info.kind;
    let method = &info.ident;

    let deps_fn = format_ident!("__overseerd_hook_{index}_deps");
    let call_fn = format_ident!("__overseerd_hook_{index}_call");
    let kind_ty_fn = format_ident!("__overseerd_hook_{index}_kind_ty");
    let descriptor_static = format_ident!("__OVERSEERD_HOOK_{index}");

    let arg_idents: Vec<_> = (0..info.params.len())
        .map(|i| format_ident!("__a{i}"))
        .collect();
    let param_tys: Vec<&Type> = info.params.iter().map(|p| &p.ty).collect();
    let param_paths: Vec<TokenStream> = info
        .params
        .iter()
        .map(|p| match &p.path {
            Some(lit) => quote!(::core::option::Option::Some(#lit)),
            None => quote!(::core::option::Option::None),
        })
        .collect();

    // Resolve the receiver only when the method takes `&self`; a self-less hook is an
    // associated call. The receiver is reached through the component source resolver fetched
    // from the erased resolver context, so the hook layer never names the container.
    let invoke = if info.takes_self {
        quote! {
            let __svc = #resolver_ctx_ext::get_resolver::<#component_source>(__ctx)
                .and_then(|__src| __src.component::<#self_ty>())
                .ok_or(#hook_error::MissingReceiver(<#self_ty as #component>::NAME))?;
            let __out = __svc.#method(#(#arg_idents),*).await;
        }
    } else {
        quote! {
            let __out = <#self_ty>::#method(#(#arg_idents),*).await;
        }
    };

    // Normalize the return into the kind's `Output`: unwrap a `Result`, boxing its error
    // into the hook layer's error (the hook layer sits below the daemon, so it cannot name
    // the daemon error — any `Error + Send + Sync` domain error boxes), then bind it at the
    // kind's `Output` type so a mismatched return is a compile error.
    let normalize = if info.is_result {
        quote! {
            let __out: <#kind as #hook_kind>::Output =
                __out.map_err(|__e| #hook_error::Other(__e.into()))?;
        }
    } else {
        quote! {
            let __out: <#kind as #hook_kind>::Output = __out;
        }
    };

    quote! {
        fn #deps_fn() -> ::std::vec::Vec<#dependency_descriptor> {
            ::std::vec![
                #( <#param_tys as #hook_param<#kind>>::dependency(#param_paths) ),*
            ]
        }

        fn #kind_ty_fn() -> #any::TypeId {
            #any::TypeId::of::<#kind>()
        }

        #[allow(unused_variables)]
        fn #call_fn<'a>(
            __ctx: &'a (dyn #resolver_ctx + ::core::marker::Send + ::core::marker::Sync),
            __cx: &'a (dyn #any::Any + ::core::marker::Send + ::core::marker::Sync),
        ) -> ::core::pin::Pin<
            ::std::boxed::Box<
                dyn ::core::future::Future<
                    Output = #hook_result<::std::boxed::Box<dyn #any::Any + ::core::marker::Send>>,
                > + ::core::marker::Send + 'a,
            >,
        > {
            ::std::boxed::Box::pin(async move {
                let __cx = __cx
                    .downcast_ref::<<#kind as #hook_kind>::Cx>()
                    .expect("hook context matches its kind");

                #(
                    let #arg_idents =
                        <#param_tys as #hook_param<#kind>>::extract(__cx, #param_paths)?;
                )*

                #invoke

                #normalize

                ::core::result::Result::Ok(
                    ::std::boxed::Box::new(__out) as ::std::boxed::Box<dyn #any::Any + ::core::marker::Send>
                )
            })
        }

        #[#distributed_slice(#hooks_slice)]
        #[linkme(crate = #linkme_crate)]
        static #descriptor_static: #hook_descriptor = #hook_descriptor {
            component_ty: #type_descriptor::of::<#self_ty>(#name),
            kind: <#kind as #hook_kind>::NAME,
            kind_ty: #kind_ty_fn,
            dependencies: #deps_fn,
            call: #call_fn,
        };
    }
}

/// Extracts the kind path from a `#[hook(Kind)]` attribute's tokens.
pub fn parse_hook_kind(attr: &syn::Attribute) -> syn::Result<Path> {
    let Meta::List(list) = &attr.meta else {
        return Err(syn::Error::new_spanned(
            attr,
            "expected #[hook(Kind)] naming a hook kind type",
        ));
    };

    syn::parse2::<Path>(list.tokens.clone())
}
