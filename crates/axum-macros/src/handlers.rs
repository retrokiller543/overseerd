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
    FnArg, GenericArgument, GenericParam, Ident, ImplItemFn, ItemImpl, LitStr, Path, PathArguments,
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

    /// Accumulated per HTTP route-attributed method (during [`ParseMethod`]).
    routes: Vec<RouteSpec>,

    /// Wire types across this block's routes (body/path/response) that must be `Dto`, collected for
    /// a single deduped assertion block (see [`client::collect_wire_types`]). Populated only with the
    /// `client` feature; emitted ungated via [`ParseMethod::client_assertions`] so it fires on native
    /// *and* wasm.
    wire_types: Vec<Type>,

    /// The `{method}_with_headers` sibling of each unary HTTP route (accumulated during
    /// [`ParseMethod`]). Header handling is HTTP-specific, so the framework core never sees it: these
    /// are rendered by [`ParseMethod::extra_client_tokens`] into an extra `impl` block, and the wasm
    /// wrapper folds the header argument onto the plain method instead.
    header_methods: Vec<ClientMethod>,

    /// Accumulated per `#[message]` ws handler method (during [`ParseMethod`]). A block is either
    /// HTTP (`routes`) or WebSocket (`ws_routes`); mixing the two is a compile error.
    ws_routes: Vec<WsRouteSpec>,

    /// `ws = P` — the WebSocket protocol this handlers block speaks, mirroring
    /// `#[controller(ws = P)]`. It selects how `#[message]` *client* methods are generated: the
    /// default (`None`) / `JsonWs` emits request/reply methods; `Stomp` emits typed `SEND` methods.
    ws_protocol: Option<syn::Path>,

    /// `codec = C` — the STOMP body codec for this block's `#[message]` SENDs (default `JsonCodec`).
    /// Encodes the payload on the generated client method and decodes it in the server handler, so
    /// the SEND path is codec-agnostic and symmetric. Ignored for a `JsonWs` block.
    ws_codec: Option<syn::Path>,
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

/// One route claimed from a method: its verb, its relative path, its own
/// [`AxumMiddleware`](../overseerd_axum/trait.AxumMiddleware.html) list (first-listed
/// outermost), and the handler closure.
struct RouteSpec {
    verb: Ident,
    path: LitStr,
    middleware: Vec<Path>,
    handler: TokenStream,
}

/// One ws message route claimed from a `#[message("dest")]` method: its destination and the
/// `Arc<Self> -> WsRoute` builder fragment.
struct WsRouteSpec {
    builder: TokenStream,
}

/// One route within a same-path group: its verb, its own middleware list, and its handler.
type RouteEntry<'a> = (&'a Ident, &'a [Path], &'a TokenStream);

impl ParseKeyed for AxumHandlers {
    fn parse_keyed(&mut self, key: &Ident, input: ParseStream) -> syn::Result<bool> {
        match key.to_string().as_str() {
            "routes_slice" => {
                eat_eq(input)?;
                self.routes_slice = Some(input.parse()?);

                Ok(true)
            }

            "ws" => {
                eat_eq(input)?;
                self.ws_protocol = Some(input.parse()?);

                Ok(true)
            }

            "codec" => {
                eat_eq(input)?;
                self.ws_codec = Some(input.parse()?);

                Ok(true)
            }

            _ => Ok(false),
        }
    }

    fn expected_keys() -> &'static [&'static str] {
        &["routes_slice", "ws", "codec"]
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
        // A `#[message("dest")]` method is a ws handler — a different surface from HTTP routes. It
        // contributes no HTTP `ClientMethod` hint (the ws client comes from the ws transport).
        if let Some(pos) = method.attrs.iter().position(route::is_message_attr) {
            let attr = method.attrs.remove(pos);
            let destination = route::parse_message_attr(&attr)?;

            let cx = self
                .context
                .as_ref()
                .expect("AxumHandlers::parse_item runs before parse_method");
            let is_pubsub = is_pubsub_protocol(self.ws_protocol.as_ref());

            // A pub/sub protocol's payload codec (block `codec = ..`, default the protocol's
            // `DefaultCodec`) drives both the server decode and the client SEND encode, so they stay
            // symmetric. A JsonWs block has no codec seam (its `WsCodec` is JSON), so pass `None`.
            let stomp_codec = is_pubsub.then(|| self.resolve_stomp_codec(&cx.paths));
            let spec = build_ws_route(
                &cx.self_ty,
                method,
                &destination,
                stomp_codec.as_ref(),
                &cx.paths,
            )?;

            // The client method's shape depends on the protocol: a pub/sub protocol emits a
            // fire-and-forget typed SEND (payload encoded via the codec); JsonWs emits a
            // request/reply call.
            let hint = if is_pubsub {
                let codec = stomp_codec.expect("pub/sub block resolves a codec");
                let protocol = self
                    .ws_protocol
                    .as_ref()
                    .expect("pub/sub block has a protocol");

                build_pubsub_send_method(
                    &method.sig.ident,
                    method,
                    &destination,
                    protocol,
                    &codec,
                    &cx.paths,
                )?
            } else {
                build_ws_client_method(
                    &cx.self_ident,
                    &method.sig.ident,
                    method,
                    &destination,
                    &cx.paths,
                )?
            };

            // A ws message payload crosses the wire just like an HTTP body, so it is `Dto` by the
            // same contract. (The reply is a protocol-specific `WsRespond` outcome, not a plain
            // payload, so only the incoming payload is asserted.)
            if let Some(payload) = ws_payload_type(method)? {
                self.wire_types.push(payload);
            }

            self.ws_routes.push(spec);

            return Ok(hint);
        }

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
            // A unary route yields the clean method (the hint the framework renders) plus its
            // `_with_headers` sibling, which we stash to render ourselves in `extra_client_tokens`.
            match client::build_client_method(
                &method.sig.ident,
                &route_attr,
                &arg_types,
                &method.sig.output,
                &cx.paths,
            ) {
                Some(methods) => {
                    self.header_methods.push(methods.with_headers);

                    Some(methods.base)
                }

                None => None,
            }
        };

        // Collect the wire types to assert `Dto` — for a plain unary route only (streaming
        // bodies/returns are framed, not single `Dto` payloads). No-op without the `client` feature.
        if stream_param.is_none() && stream_return.is_none() {
            client::collect_wire_types(&arg_types, &method.sig.output, &mut self.wire_types);
        }

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

    /// The axum extension's client-side extras: the `Dto` wire-type assertions and the wasm
    /// `#[wasm_bindgen]` binding methods. The wasm backend is chosen by the block's protocol — a
    /// STOMP block's `SEND` methods bind over the STOMP socket, an HTTP block over the fetch client,
    /// and a JsonWs request/reply block has no wasm transport yet (`None`). The extension owns this
    /// target gating, so the framework core never learns about wasm.
    fn extra_client_tokens(
        &self,
        client_ident: &Ident,
        methods: &[ClientMethod],
        paths: &Paths,
    ) -> TokenStream {
        let backend = match &self.ws_protocol {
            Some(protocol) if is_pubsub_protocol(Some(protocol)) => {
                Some(client::WasmBackend::Stomp)
            }
            // A JsonWs request/reply block: no wasm ws transport yet.
            Some(_) => None,
            None => Some(client::WasmBackend::Http),
        };

        client::extra_client_tokens(
            client_ident,
            methods,
            &self.header_methods,
            self.wire_types.clone(),
            backend,
            paths,
        )
    }
}

impl ToTokens for AxumHandlers {
    fn to_tokens(&self, out: &mut TokenStream) {
        let Some(cx) = &self.context else {
            return;
        };

        // A block is either HTTP routes or ws messages, never both — the two register into
        // different per-controller slices and assert different controller traits.
        if !self.routes.is_empty() && !self.ws_routes.is_empty() {
            out.extend(quote! {
                ::core::compile_error!(
                    "a #[handlers] block mixes HTTP route attributes with #[message] handlers; \
                     split them into separate impl blocks"
                );
            });

            return;
        }

        if !self.ws_routes.is_empty() {
            self.ws_tokens(cx, out);

            return;
        }

        if self.routes.is_empty() {
            return;
        }

        let paths = &cx.paths;
        let self_ty = &cx.self_ty;
        let axum = paths.plugin("axum");
        let app_runtime = paths.core("AppRuntime");
        let as_layer = paths.plugin("middleware::as_layer");
        let distributed_slice = paths.core("linkme::distributed_slice");
        let linkme_crate = paths.core("linkme");
        let controller_trait = paths.plugin("Controller");
        let routes_slice = self
            .routes_slice
            .clone()
            .unwrap_or_else(|| format_ident!("{}Routes", cx.self_ident));

        // Fold routes that share a relative path into one `MethodRouter`, preserving order so
        // the generated `.route(..)` calls never collide on a duplicate path within this block.
        let mut groups: Vec<(LitStr, Vec<RouteEntry>)> = Vec::new();

        for spec in &self.routes {
            let value = spec.path.value();
            let entry = (&spec.verb, spec.middleware.as_slice(), &spec.handler);

            match groups.iter_mut().find(|(path, _)| path.value() == value) {
                Some((_, entries)) => entries.push(entry),

                None => groups.push((spec.path.clone(), vec![entry])),
            }
        }

        // Each verb becomes its own `MethodRouter` so a route's own `middleware = [..]` scopes to
        // just that verb, then verbs sharing a path merge into one `MethodRouter::merge` result —
        // equivalent at runtime to chaining (`.get(..).post(..)`) when no route carries middleware.
        let layer_route = |base: TokenStream, middleware: &[Path]| -> TokenStream {
            let mut chain = base;

            for mw in middleware.iter().rev() {
                chain = quote! {
                    #chain.layer(#as_layer(
                        runtime.root().get::<#mw>().expect(
                            "middleware component missing from DI root — did you register it?",
                        ),
                    ))
                };
            }

            chain
        };

        let route_tokens = groups.iter().map(|(path, entries)| {
            let mut entries = entries.iter();
            let (first_verb, first_middleware, first_handler) =
                entries.next().expect("group has at least one route");
            let mut chain = layer_route(
                quote!(#axum::routing::#first_verb(#first_handler)),
                first_middleware,
            );

            for (verb, middleware, handler) in entries {
                let verb_router = layer_route(quote!(#axum::routing::#verb(#handler)), middleware);
                chain = quote!(#chain.merge(#verb_router));
            }

            quote!(.route(#path, #chain))
        });

        out.extend(quote! {
            const _: () = {
                // A `#[get]`/`#[post]`/… block only belongs on a REST `#[controller]`; assert it so a
                // route attribute on a `#[controller(ws = ..)]` fails clearly here, not on the
                // missing route slice.
                fn __overseerd_assert_controller<T: #controller_trait>() {}
                let _ = __overseerd_assert_controller::<#self_ty>;

                fn __overseerd_axum_route_group(
                    svc: ::std::sync::Arc<#self_ty>,
                    runtime: & #app_runtime,
                ) -> #axum::Router {
                    let _ = &svc;
                    let _ = runtime;

                    #axum::Router::new()
                        #(#route_tokens)*
                }

                #[#distributed_slice(#routes_slice)]
                #[linkme(crate = #linkme_crate)]
                static __OVERSEERD_AXUM_ROUTE_GROUP:
                    fn(::std::sync::Arc<#self_ty>, & #app_runtime) -> #axum::Router =
                    __overseerd_axum_route_group;
            };
        });
    }
}

impl AxumHandlers {
    /// Emits a ws controller's message-route group: a `fn(Arc<Self>) -> Vec<WsRoute>` builder
    /// appended to the controller's `{Controller}WsRoutes` slice, plus a `WebsocketController`
    /// assertion so a `#[message]` block on a non-ws `#[controller]` fails clearly here.
    fn ws_tokens(&self, cx: &HandlerContext, out: &mut TokenStream) {
        let paths = &cx.paths;
        let self_ty = &cx.self_ty;
        let distributed_slice = paths.core("linkme::distributed_slice");
        let linkme_crate = paths.core("linkme");
        let ws_controller_trait = paths.plugin("WebsocketController");
        let ws_route = paths.plugin("WsRoute");
        let ws_routes_slice = self
            .routes_slice
            .clone()
            .unwrap_or_else(|| format_ident!("{}WsRoutes", cx.self_ident));

        // Routes are typed to the controller's protocol, named through the `WebsocketController`
        // assoc type (the assertion below proves the bound holds).
        let ws_route_p = quote!(#ws_route<<#self_ty as #ws_controller_trait>::Protocol>);
        let builders = self.ws_routes.iter().map(|spec| &spec.builder);

        out.extend(quote! {
            const _: () = {
                fn __overseerd_assert_ws_controller<T: #ws_controller_trait>() {}
                let _ = __overseerd_assert_ws_controller::<#self_ty>;

                fn __overseerd_ws_route_group(
                    svc: ::std::sync::Arc<#self_ty>,
                ) -> ::std::vec::Vec<#ws_route_p> {
                    let _ = &svc;

                    ::std::vec![ #(#builders),* ]
                }

                #[#distributed_slice(#ws_routes_slice)]
                #[linkme(crate = #linkme_crate)]
                static __OVERSEERD_WS_ROUTE_GROUP:
                    fn(::std::sync::Arc<#self_ty>) -> ::std::vec::Vec<#ws_route_p> =
                    __overseerd_ws_route_group;
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
        middleware: route_attr.middleware.clone(),
        handler,
    })
}

/// Builds the message-route builder fragment for one `#[message("dest")]` method.
///
/// A ws handler takes `&self`, any number of `Inject<T>` parameters (resolved from the message's
/// `Request` scope — the same DI a REST route gets), and at most one *payload* parameter (any other
/// type, decoded from the frame's JSON `payload`); it returns a serializable value (encoded into the
/// reply's `ok`). The fragment evaluates — with the controller singleton `svc: Arc<Self>` in scope —
/// to a `WsRoute` whose type-erased handler resolves the injects, decodes the payload, runs the
/// method, and encodes the response.
fn build_ws_route(
    self_ty: &Type,
    method: &ImplItemFn,
    destination: &LitStr,
    stomp_codec: Option<&TokenStream>,
    paths: &Paths,
) -> syn::Result<WsRouteSpec> {
    let takes_self = matches!(method.sig.inputs.first(), Some(FnArg::Receiver(_)));

    if let Some(FnArg::Receiver(receiver)) = method.sig.inputs.first()
        && (receiver.reference.is_none() || receiver.mutability.is_some())
    {
        return Err(syn::Error::new_spanned(
            receiver,
            "ws message methods may take `&self` only (the controller singleton is shared; \
             `self` by value and `&mut self` are not allowed)",
        ));
    }

    let ws_route = paths.plugin("WsRoute");
    let ws_protocol = paths.plugin("WebsocketProtocol");
    let ws_codec = paths.plugin("WsCodec");
    let ws_respond = paths.plugin("WsRespond");
    let ws_controller_trait = paths.plugin("WebsocketController");
    let inject = paths.plugin("Inject");
    let scope_container = paths.plugin("__ScopeContainer");
    let dispatch_error = paths.plugin("WsDispatchError");

    // The controller's protocol owns the payload/outcome vocabulary and the codec. Named through
    // the `WebsocketController` assoc type so this route group works for any protocol `P` — `JsonWs`
    // decodes/encodes JSON, a STOMP protocol carries bytes + headers — with no JSON hardcoded here.
    let proto = quote!(<#self_ty as #ws_controller_trait>::Protocol);
    let payload_ty = quote!(<#proto as #ws_protocol>::Payload);
    let ws_future = paths.plugin("ws::WsFuture");
    let ws_future_p = quote!(#ws_future<#proto>);

    // Classify each typed parameter: an `Inject<T>` resolves from the per-message scope; anything
    // else is the (single) JSON payload. Build the per-arg bindings and the call list in order.
    let mut bindings: Vec<TokenStream> = Vec::new();
    let mut call_args: Vec<Ident> = Vec::new();
    let mut payload_seen = false;

    for (i, arg) in method.sig.inputs.iter().enumerate() {
        let FnArg::Typed(typed) = arg else {
            continue;
        };

        let ident = format_ident!("__a{i}");
        let ty = typed.ty.as_ref();

        match inject_inner(ty) {
            Some(handle) => bindings.push(quote! {
                let #ident = #inject(
                    __scope.extract::<#handle>().await.map_err(|__e| {
                        #dispatch_error::Inject(::std::string::ToString::to_string(&__e))
                    })?,
                );
            }),

            None => {
                if payload_seen {
                    return Err(syn::Error::new_spanned(
                        ty,
                        "a ws message method takes at most one payload parameter (the frame \
                         carries one JSON payload); other parameters must be `Inject<T>`",
                    ));
                }

                payload_seen = true;

                // A pub/sub protocol decodes the body via the block's codec (symmetric with the
                // client SEND encode) through the protocol-generic `TopicCodec<P>`; every other
                // protocol uses its `WsCodec` (JsonWs = JSON).
                let decode = match stomp_codec {
                    Some(codec) => {
                        let topic_codec_trait = paths.plugin("TopicCodec");

                        quote!(
                            <#codec as #topic_codec_trait<#proto>>::decode::<#ty>(__payload)
                                .map_err(|__e| #dispatch_error::Decode(::std::string::ToString::to_string(&__e)))?
                        )
                    }

                    None => quote!(<#proto as #ws_codec<#ty>>::decode(__payload)?),
                };

                bindings.push(quote!(let #ident: #ty = #decode;));
            }
        }

        call_args.push(ident);
    }

    // No payload parameter: the frame's payload is ignored (silence the unused binding).
    if !payload_seen {
        bindings.insert(0, quote!(let _ = &__payload;));
    }

    let method_ident = &method.sig.ident;
    let dotawait = if method.sig.asyncness.is_some() {
        quote!(.await)
    } else {
        quote!()
    };

    let invoke = if takes_self {
        quote!(<#self_ty>::#method_ident(&__svc, #(#call_args),*)#dotawait)
    } else {
        quote!(<#self_ty>::#method_ident(#(#call_args),*)#dotawait)
    };

    // The handler's raw return type — the whole value the method yields, turned into the protocol's
    // outcome by `WsRespond`. Not `client::response_type` (which peels `Result`/`Json`): a ws
    // handler returns a plain serializable value, and `respond` receives it whole.
    let response_ty = match &method.sig.output {
        ReturnType::Type(_, ty) => (**ty).clone(),

        ReturnType::Default => parse_quote!(()),
    };

    let builder = quote! {{
        let __svc = ::std::sync::Arc::clone(&svc);

        #ws_route::new(
            #destination,
            ::std::sync::Arc::new(
                move |__payload: #payload_ty, __scope: ::std::sync::Arc<#scope_container>|
                    -> #ws_future_p {
                    let __svc = ::std::sync::Arc::clone(&__svc);

                    ::std::boxed::Box::pin(async move {
                        #(#bindings)*
                        let __resp = #invoke;

                        <#proto as #ws_respond<#response_ty>>::respond(__resp)
                    })
                },
            ),
        )
    }};

    Ok(WsRouteSpec { builder })
}

/// The single non-`Inject<T>` parameter of a `#[message]` method — the payload the client method
/// takes and the frame carries. `None` for a no-payload handler; an error if more than one.
fn ws_payload_type(method: &ImplItemFn) -> syn::Result<Option<Type>> {
    let mut payload: Option<Type> = None;

    for arg in &method.sig.inputs {
        let FnArg::Typed(typed) = arg else {
            continue;
        };

        let ty = typed.ty.as_ref();

        if inject_inner(ty).is_some() {
            continue;
        }

        if payload.replace(ty.clone()).is_some() {
            return Err(syn::Error::new_spanned(
                ty,
                "a ws message method takes at most one payload parameter (the frame carries one \
                 payload); other parameters must be `Inject<T>`",
            ));
        }
    }

    Ok(payload)
}

/// Whether a `ws = P` names the STOMP protocol, by the path's last segment. Shared with the
/// controller macro (`router.rs`) to pick the wasm SEND-client backend.
/// The built-in **request/reply** WebSocket protocols, by the last segment of their path. The macro
/// can't run trait resolution to know a protocol's shape, so it treats these as request/reply and
/// **every other** `ws = P` (STOMP, WAMP, a user's own protocol) as pub/sub — so a user-defined
/// pub/sub protocol gets topic codegen with no macro change. `JsonWs` is the framework's built-in
/// request/reply protocol; a user request/reply protocol adds one entry here.
pub(crate) fn request_reply_protocols() -> &'static [&'static str] {
    &["JsonWs"]
}

/// Whether a `ws = P` block is pub/sub (topic-bearing): true for any protocol that is not a known
/// request/reply one. A `#[message]` block always names a protocol, so `None` (not a ws block) is
/// not pub/sub.
pub(crate) fn is_pubsub_protocol(protocol: Option<&syn::Path>) -> bool {
    match protocol.and_then(|path| path.segments.last()) {
        Some(segment) => !request_reply_protocols()
            .iter()
            .any(|name| segment.ident == name),

        None => false,
    }
}

impl AxumHandlers {
    /// The STOMP body codec for this block as a token stream: the `codec = ..` path, or `JsonCodec`.
    fn resolve_stomp_codec(&self, paths: &Paths) -> TokenStream {
        match &self.ws_codec {
            Some(path) => quote!(#path),

            None => {
                let json_codec = paths.plugin("JsonCodec");

                quote!(#json_codec)
            }
        }
    }
}

/// Builds the generated typed pub/sub `SEND` client method for a `#[message("dest")]` handler: a
/// fire-and-forget `fn(&self, payload) -> Result<(), ClientError<P::Status>>` bound on
/// `C: TopicSend<P>`, with the destination baked in. The payload is encoded to `P::Body` by the
/// block's `codec` (so the SEND path is codec-agnostic, matching the server decode); a no-payload
/// method sends the protocol body's default. Mirrors the JsonWs precedent ([`build_ws_client_method`]).
fn build_pubsub_send_method(
    method_ident: &Ident,
    method: &ImplItemFn,
    destination: &LitStr,
    protocol: &syn::Path,
    codec: &TokenStream,
    paths: &Paths,
) -> syn::Result<Option<ClientMethod>> {
    let payload = ws_payload_type(method)?;
    let client_error = paths.client("ClientError");
    let topic_send = paths.plugin("client::TopicSend");
    let topic_client_protocol = paths.plugin("TopicClientProtocol");
    let topic_protocol = paths.plugin("TopicProtocol");
    let topic_codec = paths.plugin("TopicCodec");

    // Encode the payload to the protocol body via the codec, or send the body's default for a
    // no-payload method.
    let (request, encode_body) = match payload {
        Some(ty) => (
            Some(ty),
            quote!(
                <#codec as #topic_codec<#protocol>>::encode(&request)
                    .map_err(|__e| #client_error::Encode(::std::string::ToString::to_string(&__e)))?
            ),
        ),
        None => (
            None,
            quote!(<<#protocol as #topic_protocol>::Body as ::core::default::Default>::default()),
        ),
    };

    Ok(Some(ClientMethod {
        ident: method_ident.clone(),
        path: String::new(),
        capability: overseerd_macros_core::client::Capability::Unary,
        request,
        encode_as: None,
        req_item: None,
        resp_item: None,
        response: parse_quote!(()),
        error_ty: None,
        extra_args: Vec::new(),
        request_envelope: None,
        request_builder: None,
        response_envelope: None,
        response_mapper: None,
        trailing_args: ::std::vec::Vec::new(),
        attrs: ::std::vec::Vec::new(),
        override_bounds: Some(quote!( C: #topic_send<#protocol> )),
        override_ret: Some(quote!(
            ::core::result::Result<(), #client_error<<#protocol as #topic_client_protocol>::Status>>
        )),
        override_body: Some(quote!({
            let __body = #encode_body;

            <C as #topic_send<#protocol>>::send(&self.0, #destination, __body).await
        })),
    }))
}

/// Builds the generated typed websocket client method for one `#[message("dest")]` handler.
/// The server route and client method share the same parameter classification: `Inject<T>` is
/// server-only, and the single non-inject parameter is the JSON payload.
fn build_ws_client_method(
    _controller: &Ident,
    method_ident: &Ident,
    method: &ImplItemFn,
    destination: &LitStr,
    paths: &Paths,
) -> syn::Result<Option<ClientMethod>> {
    let payload = ws_payload_type(method)?;

    let response = client::response_type(&method.sig.output);
    let client_error = paths.client("ClientError");
    let ws_client = paths.plugin("client::WebsocketClient");
    let ws_status = paths.plugin("client::WsStatus");
    let json_ws = paths.plugin("JsonWs");

    let (request, payload_value) = match payload {
        Some(ty) => (Some(ty), quote!(request)),
        None => (None, quote!(())),
    };
    let request_ty = request.clone().unwrap_or_else(|| syn::parse_quote!(()));

    let bounds = quote! {
        C: #ws_client<#json_ws, #request_ty, #response>
    };

    Ok(Some(ClientMethod {
        ident: method_ident.clone(),
        path: String::new(),
        capability: overseerd_macros_core::client::Capability::Unary,
        request,
        encode_as: None,
        req_item: None,
        resp_item: None,
        response: response.clone(),
        error_ty: None,
        extra_args: Vec::new(),
        request_envelope: None,
        request_builder: None,
        response_envelope: None,
        response_mapper: None,
        trailing_args: ::std::vec::Vec::new(),
        attrs: ::std::vec::Vec::new(),
        override_bounds: Some(bounds),
        override_ret: Some(quote!(
            ::core::result::Result<#response, #client_error<#ws_status>>
        )),
        override_body: Some(quote!(
            #ws_client::<#json_ws, #request_ty, #response>::websocket_call(
                &self.0,
                #destination,
                #payload_value,
            ).await
        )),
    }))
}

/// If `ty` is `Inject<H>` (the axum DI extractor), returns its inner handle type `H`; otherwise
/// `None`. Recognized by the last path segment being `Inject` with a single type argument, so it
/// matches `Inject<..>`, `axum::Inject<..>`, or the fully-qualified form alike.
fn inject_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;

    if segment.ident != "Inject" {
        return None;
    }

    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };

    args.args.iter().find_map(|arg| match arg {
        GenericArgument::Type(inner) => Some(inner),

        _ => None,
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
