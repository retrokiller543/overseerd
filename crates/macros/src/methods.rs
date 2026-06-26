//! `#[methods]` expansion (impl).
//!
//! Registers a component's lifecycle methods. Today that is the `#[init]`
//! constructor — an explicit factory that overrides the field-injection default —
//! appended to the type's `{Type}Factories` slice. Works on **any** component (a
//! `#[component]` or a `#[service]`), so a plain component gains a full-flexibility
//! constructor (sync or async, the full range of injectable parameters) without the
//! async-only `factory = ..` form. Future lifecycle attributes (e.g. start/stop
//! hooks) register through this same macro.
//!
//! The constructor is wired through the build-time `Factory` machinery: its
//! parameters self-report their dependency edges (`FromContainer`) and its return
//! value is normalized by `FactoryOutput`, so this macro emits no hand-built
//! dependency list.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ImplItem, ImplItemFn, ItemImpl, LitStr, ReturnType, Type};

use crate::attr::MethodsArgs;
use crate::hook::{self, HookInfo};
use crate::inject::{factories_slice_ident, hooks_slice_ident};
use crate::paths::overseerd_path;

pub fn expand(args: MethodsArgs, mut item: ItemImpl) -> syn::Result<TokenStream> {
    let self_ty = (*item.self_ty).clone();
    let self_ident = self_ty_ident(&self_ty)?;

    let mut init: Option<InitInfo> = None;
    let mut hooks: Vec<HookInfo> = Vec::new();

    for impl_item in &mut item.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };

        if method.attrs.iter().any(|a| a.path().is_ident("rpc")) {
            return Err(syn::Error::new_spanned(
                &method.sig,
                "#[rpc] methods belong in a #[handlers] impl, not #[methods]",
            ));
        }

        if let Some(pos) = method.attrs.iter().position(|a| a.path().is_ident("hook")) {
            let attr = method.attrs.remove(pos);
            let kind = hook::parse_hook_kind(&attr)?;

            hooks.push(hook::parse_hook(method, kind)?);

            continue;
        }

        if let Some(pos) = method.attrs.iter().position(|a| a.path().is_ident("init")) {
            method.attrs.remove(pos);

            if init.is_some() {
                return Err(syn::Error::new_spanned(
                    &method.sig,
                    "this impl block already has an #[init] constructor",
                ));
            }

            init = Some(parse_init(method)?);
        }
    }

    let factories_slice = args
        .factory_slice
        .unwrap_or_else(|| factories_slice_ident(&self_ident));

    let (marker, component) = match &init {
        Some(info) => generate_init(&self_ty, &factories_slice, info),
        None => (quote!(), quote!()),
    };

    let hooks_slice = hooks_slice_ident(&self_ident);
    let name = LitStr::new(&self_ident.to_string(), self_ident.span());
    let hook_tokens = hooks
        .iter()
        .enumerate()
        .map(|(index, info)| hook::generate_hook(&self_ty, &name, &hooks_slice, info, index));

    Ok(quote! {
        #item

        #marker

        const _: () = {
            #component

            #(#hook_tokens)*
        };
    })
}

/// The parsed shape of an `#[init]` constructor.
struct InitInfo {
    ident: syn::Ident,
    is_async: bool,
    param_types: Vec<Type>,
    output: ReturnType,
}

fn parse_init(method: &ImplItemFn) -> syn::Result<InitInfo> {
    let mut param_types = Vec::new();

    // Parameter *types* only — each must be a `FromContainer` (`Arc<T>`, `Cfg<T>`,
    // `Vec<Arc<dyn Tr>>`, a by-value injectable, …), enforced by the `Factory` impl
    // at the type level rather than by shape-matching here.
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
            }
        }
    }

    Ok(InitInfo {
        ident: method.sig.ident.clone(),
        is_async: method.sig.asyncness.is_some(),
        param_types,
        output: method.sig.output.clone(),
    })
}

/// Emits the `#[init]` factory: a fixed-name async `init` wrapper (the guard and
/// sync→async normalizer) plus its `ComponentFactoryDescriptor`, appended to the
/// type's factory slice.
fn generate_init(
    self_ty: &Type,
    factories_slice: &syn::Ident,
    info: &InitInfo,
) -> (TokenStream, TokenStream) {
    let marked = &info.ident;
    let component_construction_context = overseerd_path("ComponentConstructionContext");
    let component_factory_descriptor = overseerd_path("ComponentFactoryDescriptor");
    let dependency_descriptor = overseerd_path("DependencyDescriptor");
    let dispatch_factory = overseerd_path("dispatch_factory");
    let factory_dependencies = overseerd_path("factory_dependencies");
    let boxed_component = overseerd_path("BoxedComponent");
    let distributed_slice = overseerd_path("linkme::distributed_slice");
    let linkme_crate = overseerd_path("linkme");
    let result = overseerd_path("Result");

    // A fixed-name `init` associated fn, always `async`, forwards to the marked
    // constructor — normalizing a sync constructor to async (factories are async)
    // and serving as the compile-time uniqueness guard: two `#[init]`s in one impl
    // emit two `fn init` and fail with E0592. When the marked method is already
    // named `init`, it is its own guard and must itself be `async`.
    let fresh: Vec<_> = (0..info.param_types.len())
        .map(|i| format_ident!("__p{i}"))
        .collect();
    let param_types = &info.param_types;
    let output = &info.output;
    let dotawait = if info.is_async {
        quote!(.await)
    } else {
        quote!()
    };

    let marker = if marked == "init" {
        quote!()
    } else {
        quote! {
            impl #self_ty {
                #[doc(hidden)]
                async fn init(#(#fresh: #param_types),*) #output {
                    <#self_ty>::#marked(#(#fresh),*)#dotawait
                }
            }
        }
    };

    // The constructor is a `Factory`: it knows its parameters, so it reports its
    // own dependency edges and drives construction. No hand-built dep list — each
    // parameter's `FromContainer` impl supplies its edge.
    let component = quote! {
        fn __overseerd_init_deps() -> ::std::vec::Vec<#dependency_descriptor> {
            #factory_dependencies(<#self_ty>::init)
        }

        #[allow(unused_variables)]
        fn __overseerd_init_factory(
            cx: &mut #component_construction_context,
        ) -> ::core::pin::Pin<
            ::std::boxed::Box<
                dyn ::core::future::Future<
                    Output = #result<#boxed_component>,
                > + ::core::marker::Send + '_,
            >,
        > {
            #dispatch_factory(<#self_ty>::init, cx)
        }

        // The explicit `#[init]` factory, appended to the type's factory slice; it
        // overrides the field-injection default.
        #[#distributed_slice(#factories_slice)]
        #[linkme(crate = #linkme_crate)]
        static __OVERSEERD_INIT_FACTORY: #component_factory_descriptor =
            #component_factory_descriptor {
                construct: __overseerd_init_factory,
                dependencies: __overseerd_init_deps,
                default: false,
            };
    };

    (marker, component)
}

/// Extracts the bare identifier of the impl's self type, erroring on anything but
/// a plain path type (the same constraint `#[handlers]` enforces).
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
            "#[methods] must be applied to an inherent impl of a named type",
        )),
    }
}
