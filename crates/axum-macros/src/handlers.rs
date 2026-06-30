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
use syn::{
    FnArg, GenericArgument, GenericParam, Ident, ImplItemFn, ItemImpl, LitStr, PathArguments,
    ReturnType, Type, TypeParamBound, parse_quote,
};

use overseerd_macros_core::client::ClientMethod;
use overseerd_macros_core::extend::{ParseItem, ParseKeyed, ParseMethod, eat_eq};
use overseerd_macros_core::methods::self_ty_ident;
use overseerd_macros_core::paths::Paths;

use crate::client;
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
    /// The impl's generic type/const parameter idents, for the `use<..>` precise-capture the
    /// macro injects on streamed `impl Stream` returns (lifetimes are intentionally omitted).
    capture: Vec<Ident>,
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
        let capture = item
            .generics
            .params
            .iter()
            .filter_map(|param| match param {
                GenericParam::Type(ty) => Some(ty.ident.clone()),
                GenericParam::Const(konst) => Some(konst.ident.clone()),
                GenericParam::Lifetime(_) => None,
            })
            .collect();

        self.context = Some(HandlerContext {
            self_ty,
            self_ident,
            paths: paths.clone(),
            capture,
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

        // Claim a `#[stream]` request-body parameter (client-streaming), stripping its marker.
        let stream_param = take_stream_param(method, &cx.paths)?;

        // Classify a server-streaming return via the shorthand registry (only when not already
        // client-streaming — bidi is deferred). Read before `add_use_capture` mutates the output.
        let stream_return = if stream_param.is_some() {
            None
        } else {
            client::classify_stream_return(&method.sig.output, route_attr.streamed, &cx.paths)
        };

        // A server-streaming handler usually returns `impl Stream<..>` from `&self`; inject
        // `use<..>` so the opaque type does not capture `self`'s lifetime (edition 2024). Must run
        // before the argument types are borrowed, since it mutates the signature.
        if stream_return.is_some() {
            add_use_capture(&mut method.sig.output, &cx.capture);
        }

        let arg_types: Vec<&Type> = method
            .sig
            .inputs
            .iter()
            .filter_map(|arg| match arg {
                FnArg::Typed(typed) => Some(typed.ty.as_ref()),
                FnArg::Receiver(_) => None,
            })
            .collect();

        // Every route hands a `ClientMethod` hint to the framework's `generate_client`. The kind
        // is chosen by shape: a `#[stream]` param → client-streaming; a streamed return →
        // server-streaming; otherwise unary. Each carries the override hints its call needs.
        let hint = if let Some((index, item)) = &stream_param {
            // Path classification excludes the `#[stream]` body parameter.
            let path_args: Vec<&Type> = arg_types
                .iter()
                .enumerate()
                .filter_map(|(i, ty)| (i != *index).then_some(*ty))
                .collect();

            client::build_client_stream_method(
                &cx.self_ident,
                &method.sig.ident,
                &route_attr,
                &path_args,
                item.clone(),
                &method.sig.output,
                &cx.paths,
            )
        } else if let Some(stream) = &stream_return {
            // A known framing yields a client method; a flagged-opaque return (no decode) does not.
            match &stream.client {
                Some((wrapper_unit, item)) => client::build_stream_client_method(
                    &cx.self_ident,
                    &method.sig.ident,
                    &route_attr,
                    &arg_types,
                    wrapper_unit.clone(),
                    item.clone(),
                    &cx.paths,
                ),

                None => None,
            }
        } else {
            client::build_client_method(
                &cx.self_ident,
                &method.sig.ident,
                &route_attr,
                &arg_types,
                &method.sig.output,
                &cx.paths,
            )
        };

        let server_wrap = stream_return.as_ref().and_then(|s| s.server_wrap.as_ref());
        let in_result = stream_return.as_ref().is_some_and(|s| s.in_result);
        let spec = build_route(
            &cx.self_ty,
            method,
            &route_attr,
            stream_param.as_ref(),
            server_wrap,
            in_result,
            &cx.paths,
        )?;
        self.routes.push(spec);

        Ok(hint)
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
    route_attr: &RouteAttr,
    stream_param: Option<&(usize, Type)>,
    server_wrap: Option<&client::ServerWrap>,
    in_result: bool,
    paths: &Paths,
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

    // The closure's parameter types and the values forwarded to the handler. A `#[stream]`
    // parameter is extracted as the framework's `StreamBody<T>` (axum reads the streamed request
    // body) and handed to the handler as the deframed `impl Stream<Item = T>`.
    let stream_body = paths.plugin("StreamBody");
    let closure_params: Vec<TokenStream> = arg_types
        .iter()
        .zip(&arg_idents)
        .enumerate()
        .map(|(i, (ty, ident))| match stream_param {
            Some((index, item)) if *index == i => quote!(#ident: #stream_body<#item>),

            _ => quote!(#ident: #ty),
        })
        .collect();
    let call_args: Vec<TokenStream> = arg_idents
        .iter()
        .enumerate()
        .map(|(i, ident)| match stream_param {
            Some((index, _)) if *index == i => quote!(#ident.into_stream()),

            _ => quote!(#ident),
        })
        .collect();

    let method_ident = &method.sig.ident;
    let dotawait = if method.sig.asyncness.is_some() {
        quote!(.await)
    } else {
        quote!()
    };

    // A bare `impl Stream<..>` return is not `IntoResponse`, so the macro wraps it in the framing
    // the shorthand registry inferred. When the stream sits inside a `Result` (pre-stream
    // failure), the wrap maps over the `Result` instead. An explicit wrapper / unary body passes
    // through untouched.
    let wrap = |call: TokenStream| {
        let wrapper = match server_wrap {
            Some(client::ServerWrap::Ndjson) => {
                let ndjson = paths.plugin("Ndjson");

                quote!(#ndjson)
            }

            Some(client::ServerWrap::RawU8) => {
                let raw = paths.plugin("RawStream");
                let chunk_u8 = paths.plugin("chunk_u8");

                quote!(|__stream| #raw(#chunk_u8(__stream)))
            }

            None => return call,
        };

        if in_result {
            quote!(#call.map(#wrapper))
        } else {
            quote!((#wrapper)(#call))
        }
    };

    let handler = if takes_self {
        let call = wrap(quote!(<#self_ty>::#method_ident(&__svc, #(#call_args),*)#dotawait));

        quote! {{
            let __svc = ::std::sync::Arc::clone(&svc);

            move |#(#closure_params),*| {
                let __svc = ::std::sync::Arc::clone(&__svc);

                async move { #call }
            }
        }}
    } else {
        let call = wrap(quote!(<#self_ty>::#method_ident(#(#call_args),*)#dotawait));

        quote! {
            move |#(#closure_params),*| async move { #call }
        }
    };

    Ok(RouteSpec {
        verb: route_attr.verb.clone(),
        path: route_attr.path.clone(),
        handler,
    })
}

/// Finds and strips a `#[stream]` parameter (a streamed request body), returning its position
/// among the typed parameters (the index the closure uses) and its stream item type `T`. At most
/// one is allowed; it marks a client-streaming route.
fn take_stream_param(method: &mut ImplItemFn, paths: &Paths) -> syn::Result<Option<(usize, Type)>> {
    let mut found = None;

    for (typed_index, arg) in method
        .sig
        .inputs
        .iter_mut()
        .filter_map(|arg| match arg {
            FnArg::Typed(typed) => Some(typed),
            FnArg::Receiver(_) => None,
        })
        .enumerate()
    {
        let Some(pos) = arg.attrs.iter().position(|a| a.path().is_ident("stream")) else {
            continue;
        };

        if found.is_some() {
            return Err(syn::Error::new_spanned(
                &arg.pat,
                "a route may take at most one `#[stream]` request-body parameter",
            ));
        }

        arg.attrs.remove(pos);

        let item = client::stream_item(&arg.ty, paths).ok_or_else(|| {
            syn::Error::new_spanned(
                &arg.ty,
                "a `#[stream]` parameter must be `impl Stream<Item = T>` (or a concrete `Stream` type)",
            )
        })?;

        found = Some((typed_index, item));
    }

    Ok(found)
}

/// Injects `use<#capture>` precise capturing onto the `impl Trait` in a streamed route's return,
/// so an `impl Stream<Item = ..>` returned from an `&self` handler does not capture `self`'s
/// lifetime under edition 2024. A no-op for a concrete return type.
fn add_use_capture(output: &mut ReturnType, capture: &[Ident]) {
    if let ReturnType::Type(_, ty) = output {
        inject_capture(ty, capture);
    }
}

/// Adds `use<#capture>` to an `impl Trait` (unless already present). Lifetimes are intentionally
/// omitted — an axum response must be `'static`, so the streamed `impl Stream` must not capture
/// `self`'s lifetime; type/const params are captured (their bounds intact).
fn capture_impl_trait(impl_trait: &mut syn::TypeImplTrait, capture: &[Ident]) {
    let has_capture = impl_trait
        .bounds
        .iter()
        .any(|bound| matches!(bound, TypeParamBound::PreciseCapture(_)));

    if !has_capture {
        impl_trait.bounds.push(parse_quote!(use<#(#capture),*>));
    }
}

/// Reaches the `impl Trait` of a streamed return and captures it: a bare `impl Stream<..>`
/// directly, or the one nested in a `Wrapper<impl Stream<..>>` — descending through an outer
/// `Result<Ok, _>` first. Recursion (rather than returning a `&mut`) sidesteps the
/// conditional-reborrow the borrow checker rejects.
fn inject_capture(ty: &mut Type, capture: &[Ident]) {
    // A bare `impl Stream<Item = T>` return (no framing wrapper).
    if let Type::ImplTrait(impl_trait) = ty {
        capture_impl_trait(impl_trait, capture);

        return;
    }

    let Type::Path(type_path) = ty else {
        return;
    };
    let Some(segment) = type_path.path.segments.last_mut() else {
        return;
    };
    let is_result = segment.ident == "Result";
    let PathArguments::AngleBracketed(args) = &mut segment.arguments else {
        return;
    };
    let Some(GenericArgument::Type(inner)) = args.args.first_mut() else {
        return;
    };

    if is_result {
        // The wrapper is inside the `Ok` of a `Result<Ndjson<..>, E>` pre-stream-failure return.
        inject_capture(inner, capture);
    } else if let Type::ImplTrait(impl_trait) = inner {
        capture_impl_trait(impl_trait, capture);
    }
}
