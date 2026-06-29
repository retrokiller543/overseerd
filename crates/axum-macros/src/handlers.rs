//! The controller handlers extension: `AxumHandlers`, the [`ParseMethod`] extension that
//! makes `#[handlers]` = `MethodArgs<AxumHandlers>` (`#[methods]` + route registration).
//!
//! `AxumHandlers` claims each route-attributed method, building a typed axum handler closure
//! that resolves nothing per request beyond its extractors — the controller singleton is
//! captured once when the group is built. On emission it appends one route-group builder
//! (`fn(Arc<Self>) -> axum::Router`) to the controller's `{Controller}Routes` slice; routes
//! sharing a relative path are folded into a single `MethodRouter`. The base
//! [`MethodArgs`](overseerd_macros_core::methods::MethodArgs) still handles `#[init]`/`#[hook]`.

use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::parse::ParseStream;
use syn::{FnArg, Ident, ImplItemFn, ItemImpl, LitStr, Type};

use overseerd_macros_core::client::ClientMethod;
use overseerd_macros_core::extend::{ParseItem, ParseKeyed, ParseMethod, eat_eq};
use overseerd_macros_core::methods::self_ty_ident;
use overseerd_macros_core::paths::Paths;

use crate::route::{self, RouteAttr};

/// The controller handlers extension. Accumulates the impl's route specs and the captured impl
/// context, then emits a single route-group builder appended to the controller's route slice.
#[derive(Default)]
pub struct AxumHandlers {
    /// `routes_slice = ..` — the per-controller slice to append to (default `{Controller}Routes`).
    routes_slice: Option<Ident>,

    /// Captured during [`ParseItem`]: the impl's self type and resolved paths.
    context: Option<HandlerContext>,

    /// Accumulated per route-attributed method (during [`ParseMethod`]).
    routes: Vec<RouteSpec>,
}

/// The impl context `AxumHandlers` needs to emit (captured in the item pass).
struct HandlerContext {
    self_ty: Type,
    self_ident: Ident,
    paths: Paths,
}

/// One route claimed from a method: its verb, its relative path, and the handler closure.
struct RouteSpec {
    verb: Ident,
    path: LitStr,
    handler: TokenStream,
}

impl ParseKeyed for AxumHandlers {
    fn parse_keyed(&mut self, key: &Ident, input: ParseStream) -> syn::Result<bool> {
        match key.to_string().as_str() {
            "routes_slice" => {
                eat_eq(input)?;
                self.routes_slice = Some(input.parse()?);

                Ok(true)
            }

            _ => Ok(false),
        }
    }

    fn expected_keys() -> &'static [&'static str] {
        &["routes_slice"]
    }
}

impl ParseItem<ItemImpl> for AxumHandlers {
    fn parse_item(&mut self, item: &ItemImpl, paths: &Paths) -> syn::Result<()> {
        let self_ty = (*item.self_ty).clone();
        let self_ident = self_ty_ident(&self_ty)?;

        self.context = Some(HandlerContext {
            self_ty,
            self_ident,
            paths: paths.clone(),
        });

        Ok(())
    }
}

impl ParseMethod for AxumHandlers {
    fn parse_method(&mut self, method: &mut ImplItemFn) -> syn::Result<Option<ClientMethod>> {
        let Some(pos) = method.attrs.iter().position(route::is_route_attr) else {
            return Ok(None);
        };

        let attr = method.attrs.remove(pos);
        let route_attr = route::parse_route_attr(&attr)?;

        // `parse_item` runs before the method walk, so the context is always present.
        let cx = self
            .context
            .as_ref()
            .expect("AxumHandlers::parse_item runs before parse_method");

        let spec = build_route(&cx.self_ty, method, route_attr)?;
        self.routes.push(spec);

        // No client hint in v1 — the HTTP client needs verb+path mapping and is deferred.
        Ok(None)
    }
}

impl ToTokens for AxumHandlers {
    fn to_tokens(&self, out: &mut TokenStream) {
        let Some(cx) = &self.context else {
            return;
        };

        if self.routes.is_empty() {
            return;
        }

        let paths = &cx.paths;
        let self_ty = &cx.self_ty;
        let axum = paths.plugin("axum");
        let distributed_slice = paths.core("linkme::distributed_slice");
        let linkme_crate = paths.core("linkme");
        let routes_slice = self
            .routes_slice
            .clone()
            .unwrap_or_else(|| format_ident!("{}Routes", cx.self_ident));

        // Fold routes that share a relative path into one `MethodRouter`, preserving order so
        // the generated `.route(..)` calls never collide on a duplicate path within this block.
        let mut groups: Vec<(LitStr, Vec<(&Ident, &TokenStream)>)> = Vec::new();

        for spec in &self.routes {
            let value = spec.path.value();

            match groups.iter_mut().find(|(path, _)| path.value() == value) {
                Some((_, entries)) => entries.push((&spec.verb, &spec.handler)),

                None => groups.push((spec.path.clone(), vec![(&spec.verb, &spec.handler)])),
            }
        }

        let route_tokens = groups.iter().map(|(path, entries)| {
            let mut entries = entries.iter();
            let (first_verb, first_handler) = entries.next().expect("group has at least one route");
            let mut chain = quote!(#axum::routing::#first_verb(#first_handler));

            for (verb, handler) in entries {
                chain = quote!(#chain.#verb(#handler));
            }

            quote!(.route(#path, #chain))
        });

        out.extend(quote! {
            const _: () = {
                fn __overseerd_axum_route_group(
                    svc: ::std::sync::Arc<#self_ty>,
                ) -> #axum::Router {
                    let _ = &svc;

                    #axum::Router::new()
                        #(#route_tokens)*
                }

                #[#distributed_slice(#routes_slice)]
                #[linkme(crate = #linkme_crate)]
                static __OVERSEERD_AXUM_ROUTE_GROUP: fn(::std::sync::Arc<#self_ty>) -> #axum::Router =
                    __overseerd_axum_route_group;
            };
        });
    }
}

/// Builds the typed axum handler closure for one route-attributed method.
///
/// The closure declares the method's own parameters (all axum extractors — `Json`, `Path`,
/// `Inject<..>`, …) so axum drives extraction, captures the controller singleton by `Arc`, and
/// forwards to the method. A `&self` method is called with the captured singleton; a method
/// without a receiver is called associated.
fn build_route(
    self_ty: &Type,
    method: &ImplItemFn,
    route_attr: RouteAttr,
) -> syn::Result<RouteSpec> {
    let takes_self = match method.sig.inputs.first() {
        Some(FnArg::Receiver(receiver)) => {
            if receiver.reference.is_none() || receiver.mutability.is_some() {
                return Err(syn::Error::new_spanned(
                    receiver,
                    "controller route methods may take `&self` only (the controller singleton \
                     is shared; `self` by value and `&mut self` are not allowed)",
                ));
            }

            true
        }

        _ => false,
    };

    let arg_types: Vec<&Type> = method
        .sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(typed) => Some(typed.ty.as_ref()),
            FnArg::Receiver(_) => None,
        })
        .collect();

    let arg_idents: Vec<Ident> = (0..arg_types.len())
        .map(|i| format_ident!("__a{i}"))
        .collect();
    let method_ident = &method.sig.ident;
    let dotawait = if method.sig.asyncness.is_some() {
        quote!(.await)
    } else {
        quote!()
    };

    let handler = if takes_self {
        let call = quote!(<#self_ty>::#method_ident(&__svc, #(#arg_idents),*)#dotawait);

        quote! {{
            let __svc = ::std::sync::Arc::clone(&svc);

            move |#(#arg_idents: #arg_types),*| {
                let __svc = ::std::sync::Arc::clone(&__svc);

                async move { #call }
            }
        }}
    } else {
        let call = quote!(<#self_ty>::#method_ident(#(#arg_idents),*)#dotawait);

        quote! {
            move |#(#arg_idents: #arg_types),*| async move { #call }
        }
    };

    Ok(RouteSpec {
        verb: route_attr.verb,
        path: route_attr.path,
        handler,
    })
}
