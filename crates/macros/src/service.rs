//! `#[service]` expansion.
//!
//! Collects the `#[rpc]` methods of an inherent `impl` into a `ServiceDescriptor`
//! plus per-method erased handler wrappers. A service may be:
//!
//! - **stateless** — no method takes `self`; handlers are plain associated fns.
//! - **stateful** — methods take `&self`; the type is a `Singleton` component.
//!   Each call resolves the singleton from the container and invokes the method.
//!   The instance is built either by an `#[init]` constructor (whose `Arc<T>`
//!   parameters become injected dependencies) or supplied via
//!   `DaemonBuilder::with_component`.
//!
//! All generated items live in a `const _: () = { ... }` block so the mangled
//! wrappers and statics never leak; `inventory::submit!` registers the service
//! (and, when there is an `#[init]`, the component) for `auto_discover`.

use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{FnArg, ImplItem, ImplItemFn, ItemImpl, LitStr, Meta, Type, spanned::Spanned};

use crate::attr::{self, ServiceArgs};

pub fn expand(args: ServiceArgs, mut item: ItemImpl) -> syn::Result<TokenStream> {
    let self_ty = (*item.self_ty).clone();
    let self_ident = self_ty_ident(&self_ty)?;
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

    let mut wrappers = Vec::new();
    let mut descriptors = Vec::new();
    let mut init: Option<InitInfo> = None;

    for impl_item in &mut item.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };

        if let Some(pos) = method.attrs.iter().position(|a| a.path().is_ident("init")) {
            method.attrs.remove(pos);

            if init.is_some() {
                return Err(syn::Error::new_spanned(
                    &method.sig,
                    "a service may have at most one #[init] constructor",
                ));
            }

            init = Some(parse_init(method)?);
            continue;
        }

        let Some(pos) = method.attrs.iter().position(|a| a.path().is_ident("rpc")) else {
            continue;
        };

        let rpc_attr = method.attrs.remove(pos);
        let rpc_args = match &rpc_attr.meta {
            Meta::Path(_) => attr::RpcArgs { operation: None },
            _ => rpc_attr.parse_args::<attr::RpcArgs>()?,
        };

        let (wrapper, descriptor) =
            expand_method(&self_ty, &self_ident, &self_name, method, rpc_args)?;

        wrappers.push(wrapper);
        descriptors.push(descriptor);
    }

    if descriptors.is_empty() {
        return Err(syn::Error::new_spanned(
            &item,
            "#[service] impl block has no #[rpc] methods",
        ));
    }

    let component = match &init {
        Some(info) => generate_component(&self_ty, &self_ident, &self_name, &id, &name, info),
        None => quote!(),
    };

    let count = descriptors.len();
    let rpcs_static = format_ident!("__OVERSEER_RPCS_{}", self_ident.to_string().to_uppercase());
    let service_static =
        format_ident!("__OVERSEER_SERVICE_{}", self_ident.to_string().to_uppercase());

    Ok(quote! {
        #item

        const _: () = {
            #(#wrappers)*

            #component

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

/// Builds the erased handler wrapper and `RpcDescriptor` for one `#[rpc]` method.
fn expand_method(
    self_ty: &Type,
    self_ident: &syn::Ident,
    self_name: &LitStr,
    method: &ImplItemFn,
    rpc_args: attr::RpcArgs,
) -> syn::Result<(TokenStream, TokenStream)> {
    if method.sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            &method.sig,
            "rpc methods must be `async`",
        ));
    }

    let takes_self = match method.sig.inputs.first() {
        Some(FnArg::Receiver(receiver)) => {
            if receiver.reference.is_none() || receiver.mutability.is_some() {
                return Err(syn::Error::new_spanned(
                    receiver,
                    "rpc methods may take `&self` only (the service singleton is shared; \
                     `self` by value and `&mut self` are not allowed)",
                ));
            }

            true
        }
        _ => false,
    };

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
    let ret = handler_return_type();

    let wrapper = if takes_self {
        let param_types: Vec<&Type> = method
            .sig
            .inputs
            .iter()
            .filter_map(|arg| match arg {
                FnArg::Typed(typed) => Some(typed.ty.as_ref()),
                FnArg::Receiver(_) => None,
            })
            .collect();
        let arg_idents: Vec<_> = (0..param_types.len())
            .map(|i| format_ident!("__a{i}"))
            .collect();

        quote! {
            fn #wrapper_ident(ctx: ::overseer_core::RpcCallContext) -> #ret {
                ::std::boxed::Box::pin(async move {
                    let __svc = ctx
                        .component::<#self_ty>()
                        .ok_or(::overseer_core::Error::MissingComponent(#self_name))?;

                    ::overseer_core::dispatch_with(
                        move |#(#arg_idents: #param_types),*| {
                            let __svc = ::std::sync::Arc::clone(&__svc);

                            async move { <#self_ty>::#method_ident(&__svc, #(#arg_idents),*).await }
                        },
                        ctx,
                    )
                    .await
                })
            }
        }
    } else {
        quote! {
            fn #wrapper_ident(ctx: ::overseer_core::RpcCallContext) -> #ret {
                ::overseer_core::dispatch_with(<#self_ty>::#method_ident, ctx)
            }
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

/// The `#[init]` constructor: its `Arc<T>` parameters are injected dependencies.
struct InitInfo {
    ident: syn::Ident,
    is_async: bool,
    fallible: bool,
    dependencies: Vec<Type>,
}

fn parse_init(method: &ImplItemFn) -> syn::Result<InitInfo> {
    let mut dependencies = Vec::new();

    for arg in &method.sig.inputs {
        match arg {
            FnArg::Receiver(receiver) => {
                return Err(syn::Error::new_spanned(
                    receiver,
                    "#[init] is a constructor and cannot take `self`",
                ));
            }
            FnArg::Typed(typed) => dependencies.push(attr::arc_inner_type(&typed.ty)?),
        }
    }

    Ok(InitInfo {
        ident: method.sig.ident.clone(),
        is_async: method.sig.asyncness.is_some(),
        fallible: attr::returns_result(&method.sig.output),
        dependencies,
    })
}

/// Generates the singleton `ComponentDescriptor`, its factory (constructor
/// injection from the `#[init]` parameters), and its `inventory` registration.
fn generate_component(
    self_ty: &Type,
    self_ident: &syn::Ident,
    self_name: &LitStr,
    id: &LitStr,
    name: &LitStr,
    init: &InitInfo,
) -> TokenStream {
    let init_ident = &init.ident;

    let resolved = init.dependencies.iter().map(|dep| {
        let dep_name = LitStr::new(&dep.to_token_stream().to_string(), dep.span());

        quote! {
            cx.resolve::<#dep>()
                .ok_or(::overseer_core::Error::MissingComponent(#dep_name))?
        }
    });

    let mut call = quote!(<#self_ty>::#init_ident(#(#resolved),*));

    if init.is_async {
        call = quote!(#call.await);
    }

    if init.fallible {
        call = quote!(#call?);
    }

    let dependency_descriptors = init.dependencies.iter().map(|dep| {
        let dep_name = LitStr::new(&dep.to_token_stream().to_string(), dep.span());

        quote! {
            ::overseer_core::DependencyDescriptor {
                name: #dep_name,
                ty: ::overseer_core::TypeDescriptor::of::<#dep>(#dep_name),
                optional: false,
            }
        }
    });
    let dependency_count = init.dependencies.len();

    let factory_ident = format_ident!("__overseer_factory_{}", self_ident.to_string().to_lowercase());
    let deps_static = format_ident!("__OVERSEER_DEPS_{}", self_ident.to_string().to_uppercase());
    let component_static =
        format_ident!("__OVERSEER_COMPONENT_{}", self_ident.to_string().to_uppercase());

    quote! {
        fn #factory_ident(
            cx: &mut ::overseer_core::ComponentConstructionContext,
        ) -> ::core::pin::Pin<
            ::std::boxed::Box<
                dyn ::core::future::Future<
                    Output = ::overseer_core::Result<::overseer_core::BoxedComponent>,
                > + ::core::marker::Send + '_,
            >,
        > {
            ::std::boxed::Box::pin(async move {
                let __instance = #call;

                ::core::result::Result::Ok(::overseer_core::BoxedComponent {
                    ty: ::overseer_core::TypeDescriptor::of::<#self_ty>(#self_name),
                    value: ::std::boxed::Box::new(::std::sync::Arc::new(__instance)),
                })
            })
        }

        static #deps_static: [::overseer_core::DependencyDescriptor; #dependency_count] = [
            #(#dependency_descriptors),*
        ];

        static #component_static: ::overseer_core::ComponentDescriptor =
            ::overseer_core::ComponentDescriptor {
                id: #id,
                name: #name,
                ty: ::overseer_core::TypeDescriptor::of::<#self_ty>(#self_name),
                scope: ::overseer_core::ComponentScope::Singleton,
                dependencies: &#deps_static,
                factory: #factory_ident,
            };

        ::overseer_core::inventory::submit! {
            ::overseer_core::Descriptor::Component(&#component_static)
        }
    }
}

/// The erased `RpcHandler` return type, repeated by both wrapper forms.
fn handler_return_type() -> TokenStream {
    quote! {
        ::core::pin::Pin<
            ::std::boxed::Box<
                dyn ::core::future::Future<
                    Output = ::overseer_core::Result<::overseer_core::RpcResponse>,
                > + ::core::marker::Send,
            >,
        >
    }
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