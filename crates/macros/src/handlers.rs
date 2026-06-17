//! `#[handlers]` expansion (impl).
//!
//! Contributes the impl's `#[rpc]` methods to the service of `Self` (as an
//! `RpcGroup` keyed by type, so multiple impls merge into one service), and
//! turns an optional `#[init]` constructor into an explicit singleton factory.
//!
//! The `#[init]` constructor (any name) gets a fixed-name `init` associated fn
//! generated on the type that forwards the injected dependencies to it
//! (constructor injection). That fixed name is also a compile-time guard: two
//! `#[init]`s anywhere on the type produce two `init` definitions and fail with
//! E0592 ("duplicate definitions with name `init`"). When the marked method is
//! itself named `init`, it serves as its own marker and no wrapper is emitted.

use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{
    FnArg, ImplItem, ImplItemFn, ItemImpl, LitStr, Meta, ReturnType, Type, spanned::Spanned,
};

use crate::{attr, paths::overseer_path};

pub fn expand(mut item: ItemImpl) -> syn::Result<TokenStream> {
    let self_ty = (*item.self_ty).clone();
    let self_ident = self_ty_ident(&self_ty)?;
    let self_name = LitStr::new(&self_ident.to_string(), self_ident.span());

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
                    "this impl block already has an #[init] constructor",
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

    let rpc_registration = if descriptors.is_empty() {
        quote!()
    } else {
        let count = descriptors.len();
        let descriptor = overseer_path("Descriptor");
        let inventory_submit = overseer_path("inventory::submit");
        let rpc_descriptor = overseer_path("RpcDescriptor");
        let rpc_group = overseer_path("RpcGroup");
        let type_descriptor = overseer_path("TypeDescriptor");

        quote! {
            static __OVERSEER_RPCS: [#rpc_descriptor; #count] = [
                #(#descriptors),*
            ];

            static __OVERSEER_RPC_GROUP: #rpc_group = #rpc_group {
                service: #type_descriptor::of::<#self_ty>(#self_name),
                rpcs: &__OVERSEER_RPCS,
            };

            #inventory_submit! {
                #descriptor::Rpcs(&__OVERSEER_RPC_GROUP)
            }
        }
    };

    let (init_marker, init_component) = match &init {
        Some(info) => generate_init(&self_ty, &self_name, info),
        None => (quote!(), quote!()),
    };

    Ok(quote! {
        #item

        #init_marker

        const _: () = {
            #(#wrappers)*

            #init_component

            #rpc_registration
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
    let dispatch_with = overseer_path("dispatch_with");
    let error = overseer_path("Error");
    let operation_kind = overseer_path("OperationKind");
    let rpc_call_context = overseer_path("RpcCallContext");
    let rpc_descriptor = overseer_path("RpcDescriptor");
    let type_descriptor = overseer_path("TypeDescriptor");
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
            fn #wrapper_ident(ctx: #rpc_call_context) -> #ret {
                ::std::boxed::Box::pin(async move {
                    let __svc = ctx
                        .component::<#self_ty>()
                        .ok_or(#error::MissingComponent(#self_name))?;

                    #dispatch_with(
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
            fn #wrapper_ident(ctx: #rpc_call_context) -> #ret {
                #dispatch_with(<#self_ty>::#method_ident, ctx)
            }
        }
    };

    let descriptor = quote! {
        #rpc_descriptor {
            name: #method_name,
            operation: #operation_kind::#operation,
            parameters: &[],
            output: #type_descriptor::of::<#output_ty>(#output_name),
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
    param_types: Vec<Type>,
    dep_types: Vec<Type>,
    output: ReturnType,
}

fn parse_init(method: &ImplItemFn) -> syn::Result<InitInfo> {
    let mut param_types = Vec::new();
    let mut dep_types = Vec::new();

    for arg in &method.sig.inputs {
        match arg {
            FnArg::Receiver(receiver) => {
                return Err(syn::Error::new_spanned(
                    receiver,
                    "#[init] is a constructor and cannot take `self`",
                ));
            }
            FnArg::Typed(typed) => {
                param_types.push((*typed.ty).clone());
                dep_types.push(attr::arc_inner_type(&typed.ty)?);
            }
        }
    }

    Ok(InitInfo {
        ident: method.sig.ident.clone(),
        is_async: method.sig.asyncness.is_some(),
        fallible: attr::returns_result(&method.sig.output),
        param_types,
        dep_types,
        output: method.sig.output.clone(),
    })
}

/// Generates the fixed-name `init` marker/wrapper (module scope) and the
/// singleton component factory (const-block scope).
fn generate_init(
    self_ty: &Type,
    self_name: &LitStr,
    info: &InitInfo,
) -> (TokenStream, TokenStream) {
    let marked = &info.ident;
    let boxed_component = overseer_path("BoxedComponent");
    let component_construction_context = overseer_path("ComponentConstructionContext");
    let component_descriptor = overseer_path("ComponentDescriptor");
    let component_scope = overseer_path("ComponentScope");
    let dependency_descriptor = overseer_path("DependencyDescriptor");
    let descriptor = overseer_path("Descriptor");
    let error = overseer_path("Error");
    let inventory_submit = overseer_path("inventory::submit");
    let result = overseer_path("Result");
    let type_descriptor = overseer_path("TypeDescriptor");

    // The fixed `init` name is the compile-time uniqueness guard. If the marked
    // method is already named `init`, it is its own marker and needs no wrapper.
    let marker = if marked == "init" {
        quote!()
    } else {
        let fresh: Vec<_> = (0..info.param_types.len())
            .map(|i| format_ident!("__p{i}"))
            .collect();
        let param_types = &info.param_types;
        let output = &info.output;
        let asyncness = if info.is_async {
            quote!(async)
        } else {
            quote!()
        };
        let dotawait = if info.is_async {
            quote!(.await)
        } else {
            quote!()
        };

        quote! {
            impl #self_ty {
                #[doc(hidden)]
                #asyncness fn init(#(#fresh: #param_types),*) #output {
                    <#self_ty>::#marked(#(#fresh),*)#dotawait
                }
            }
        }
    };

    let resolved = info.dep_types.iter().map(|t| {
        let dep_name = LitStr::new(&t.to_token_stream().to_string(), t.span());

        quote! {
            cx.resolve::<#t>()
                .ok_or(#error::MissingComponent(#dep_name))?
        }
    });

    let mut call = quote!(<#self_ty>::init(#(#resolved),*));

    if info.is_async {
        call = quote!(#call.await);
    }

    if info.fallible {
        call = quote!(#call?);
    }

    let dependency_descriptors = info.dep_types.iter().map(|t| {
        let dep_name = LitStr::new(&t.to_token_stream().to_string(), t.span());

        quote! {
            #dependency_descriptor {
                name: #dep_name,
                ty: #type_descriptor::of::<#t>(#dep_name),
                optional: false,
            }
        }
    });
    let dependency_count = info.dep_types.len();

    let component = quote! {
        #[allow(unused_variables)]
        fn __overseer_init_factory(
            cx: &mut #component_construction_context,
        ) -> ::core::pin::Pin<
            ::std::boxed::Box<
                dyn ::core::future::Future<
                    Output = #result<#boxed_component>,
                > + ::core::marker::Send + '_,
            >,
        > {
            ::std::boxed::Box::pin(async move {
                let __instance = #call;

                ::core::result::Result::Ok(#boxed_component {
                    ty: #type_descriptor::of::<#self_ty>(#self_name),
                    value: ::std::boxed::Box::new(::std::sync::Arc::new(__instance)),
                })
            })
        }

        static __OVERSEER_INIT_DEPS: [#dependency_descriptor; #dependency_count] = [
            #(#dependency_descriptors),*
        ];

        static __OVERSEER_INIT_COMPONENT: #component_descriptor =
            #component_descriptor {
                id: #self_name,
                name: #self_name,
                ty: #type_descriptor::of::<#self_ty>(#self_name),
                scope: #component_scope::Singleton,
                dependencies: &__OVERSEER_INIT_DEPS,
                factory: ::core::option::Option::Some(__overseer_init_factory),
                default_factory: false,
            };

        #inventory_submit! {
            #descriptor::Component(&__OVERSEER_INIT_COMPONENT)
        }
    };

    (marker, component)
}

/// The erased `RpcHandler` return type, repeated by both wrapper forms.
fn handler_return_type() -> TokenStream {
    let result = overseer_path("Result");
    let rpc_response = overseer_path("RpcResponse");

    quote! {
        ::core::pin::Pin<
            ::std::boxed::Box<
                dyn ::core::future::Future<
                    Output = #result<#rpc_response>,
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
            "#[handlers] must be applied to an impl of a named type",
        )),
    }
}
