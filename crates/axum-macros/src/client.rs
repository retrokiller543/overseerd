//! Building the protocol-agnostic [`ClientMethod`] hint for an HTTP route.
//!
//! The framework owns client generation (in `macros-core`); a protocol only describes each
//! method as a hint. For HTTP that means: classify the handler's extractor args into client
//! inputs (the `Path` params, a `Json`/`Form` body) vs server-only ones (`Inject`/`State`/
//! `Extension`, dropped), then fill the hint so the generated method is ergonomic — the body is
//! the raw `T` (not `Json<T>`), and the path holes are dedicated typed params (or a tuple when
//! there are many) named after the route. The `request_builder` re-wraps those into an
//! `http::Request` with the verb, the `BASE`+route URI, and the typed body.
//!
//! Custom `FromRequestParts` guards (auth, tenant, …) count as server-only context and are
//! dropped like `Inject`, so a guarded route still gets a client method. Only a route using an
//! extractor that consumes wire data the client still can't encode ([`UNSUPPORTED_WIRE`], the
//! whole-request extractors) opts out: it yields `None`, the server route still registers, and it
//! simply gets no client method (rather than a silently wrong one).

use overseerd_macros_core::attr::{first_type_arg, type_name};
use overseerd_macros_core::client::{Capability, ClientMethod};
use overseerd_macros_core::paths::Paths;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{GenericArgument, Ident, PathArguments, ReturnType, Type, TypeParamBound};

use crate::route::RouteAttr;

/// The classified client inputs of a route: an optional `Path` type, an optional query, and an
/// optional request body. Shared by the unary and streaming method builders.
struct Inputs {
    path_ty: Option<Type>,
    query: Option<QueryInput>,
    body: Option<Body>,
}

/// A route's query-string input: a typed `Query<T>` (URL-encoded from the `Dto` `T`) or the untyped
/// `RawQuery` (the raw query string, passed through as an `Option<String>`).
#[allow(clippy::large_enum_variant)]
enum QueryInput {
    Typed(Type),
    Raw,
}

/// A route's request body: which [`HttpBody`](../overseerd_axum/client/trait.HttpBody.html) wrapper
/// carries it, and — for the serde-typed bodies — the payload type. The wrapper-typed bodies
/// (`Bytes`/`RawForm`/`Multipart`) have a fixed client parameter type, so they carry no `inner`.
struct Body {
    kind: BodyKind,
    inner: Option<Type>,
}

/// Classifies a route's handler arguments into client inputs, dropping server-only extractors.
///
/// The client encodes the wire inputs it recognizes — `Path` params, a query (`Query<T>`/`RawQuery`),
/// and a body (`Json`/`Form`/`Bytes`/`RawForm`/`Multipart`). Everything else is treated as
/// **server-only request context** — the framework's `Inject`/`State`/`Extension`/`ConnectInfo`, *and*
/// any custom [`FromRequestParts`] guard (an auth check, a tenant resolver, …). A guard authorizes or
/// contextualizes the request server-side and carries nothing over the wire, so it is simply dropped
/// from the client signature, exactly like `Inject`.
///
/// A route opts out entirely (`None`) only when it can't be represented faithfully: two of the same
/// slot (e.g. two bodies), or an extractor in [`UNSUPPORTED_WIRE`] whose wire data the client still
/// can't encode. Opting out beats emitting a method that would silently drop data.
///
/// [`FromRequestParts`]: https://docs.rs/axum/latest/axum/extract/trait.FromRequestParts.html
fn classify(arg_types: &[&Type]) -> Option<Inputs> {
    let mut path_ty: Option<Type> = None;
    let mut query: Option<QueryInput> = None;
    let mut body: Option<Body> = None;

    // Records a body slot, refusing a second one (a route has at most one body).
    let set_body = |slot: &mut Option<Body>, kind, inner| {
        if slot.is_some() {
            return None;
        }

        *slot = Some(Body { kind, inner });

        Some(())
    };

    for ty in arg_types {
        if let Some(inner) = first_type_arg(ty, "Path") {
            if path_ty.is_some() {
                return None;
            }

            path_ty = Some(inner);

            continue;
        }

        if let Some(inner) = first_type_arg(ty, "Query") {
            if query.is_some() {
                return None;
            }

            query = Some(QueryInput::Typed(inner));

            continue;
        }

        if let Some(inner) = first_type_arg(ty, "Json") {
            set_body(&mut body, BodyKind::Json, Some(inner))?;

            continue;
        }

        if let Some(inner) = first_type_arg(ty, "Form") {
            set_body(&mut body, BodyKind::Form, Some(inner))?;

            continue;
        }

        // The wrapper-typed bodies and the raw query are matched by name (no type argument).
        match type_name(ty).map(Ident::to_string).as_deref() {
            Some("RawQuery") => {
                if query.is_some() {
                    return None;
                }

                query = Some(QueryInput::Raw);
            }

            Some("Bytes") => set_body(&mut body, BodyKind::Bytes, None)?,
            Some("RawForm") => set_body(&mut body, BodyKind::RawForm, None)?,
            Some("Multipart") => set_body(&mut body, BodyKind::Multipart, None)?,

            // An extractor carrying wire data the client can't encode yet: opt the whole route out
            // rather than emit a method that silently drops it.
            Some(name) if UNSUPPORTED_WIRE.contains(&name) => return None,

            // Anything else — DI/state or a custom `FromRequestParts` guard — is server-only request
            // context. It never crosses the wire, so drop it from the client signature.
            _ => {}
        }
    }

    Some(Inputs {
        path_ty,
        query,
        body,
    })
}

/// Collects the **wire** types of a route that must implement [`Dto`](../overseerd_axum/trait.Dto.html)
/// — the path parameter(s), a typed `Query<T>`, a `Json`/`Form` request body, and the response.
/// Pushed into `sink` (across a whole `#[handlers]` block) so the caller can dedupe and emit a single
/// assertion block. Only with the `client` feature (a `Dto` is what the client builds a typed method
/// from); a server-only build leaves plain `Serialize` types be.
///
/// Server-only extractors and the non-serde bodies (`Bytes`/`RawForm`/`Multipart`, whose client
/// parameter is a raw `Vec<u8>` or the `Multipart` builder) contribute nothing — only what the client
/// serializes as a `Dto` is asserted.
pub fn collect_wire_types(arg_types: &[&Type], output: &ReturnType, sink: &mut Vec<Type>) {
    if !cfg!(feature = "client") {
        return;
    }

    let Some(inputs) = classify(arg_types) else {
        return;
    };

    if let Some(path_ty) = inputs.path_ty {
        sink.push(path_ty);
    }

    if let Some(QueryInput::Typed(query)) = inputs.query {
        sink.push(query);
    }

    // Only the serde-typed bodies are `Dto`; the wrapper-typed ones carry no `inner`.
    if let Some(Body {
        inner: Some(body), ..
    }) = inputs.body
    {
        sink.push(body);
    }

    // The response payload (a `Result`/`Json` return is peeled). `()` is a `Dto`, so a bare return
    // is fine; a plain unary response payload is asserted like any other wire type.
    sink.push(response_type(output));
}

/// Emits a **single** `const` block asserting every collected wire type is [`Dto`], turning a
/// forgotten `#[dto]` into a clear "`X: Dto` is not satisfied" error rather than a cascade of
/// serde/`IntoResponse` failures. The types are deduped by their token text, so a type shared across
/// routes is asserted once. Empty when nothing was collected (no client feature, or no routes).
pub fn dto_assertions(mut wire_types: Vec<Type>, paths: &Paths) -> TokenStream {
    if wire_types.is_empty() {
        return quote!();
    }

    // Dedupe by textual form (`Type` is not `Hash`/`Eq`), so each distinct type is asserted once.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    wire_types.retain(|ty| seen.insert(quote!(#ty).to_string()));

    let dto = paths.plugin("Dto");
    let asserts = wire_types
        .iter()
        .map(|ty| quote!(__overseerd_assert_dto::<#ty>();));

    quote! {
        const _: () = {
            fn __overseerd_assert_dto<T: #dto>() {}

            fn __overseerd_assert_wire_types() {
                #(#asserts)*
            }
        };
    }
}

/// The hidden Rust name of the wasm wrapper newtype for a client, e.g. `__GreetControllerClientWasm`.
fn wasm_wrapper_ident(client_ident: &Ident) -> Ident {
    format_ident!("__{}Wasm", client_ident)
}

/// Which shared transport a generated wasm client binds to — chosen by the controller's protocol.
/// It selects the concrete backend, how the client is built from the [`Connection`], and each
/// method's reply shape (an HTTP unary returns an `HttpResponse` body; a STOMP `SEND` returns unit).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WasmBackend {
    /// HTTP controllers over the `reqwest` fetch client; methods return the decoded response body.
    Http,
    /// `#[controller(ws = Stomp)]` `SEND` clients over the STOMP socket; methods return `void`.
    Stomp,
}

impl WasmBackend {
    /// The feature gating this backend's wasm binding (its transport must be compiled in).
    fn available(self) -> bool {
        match self {
            WasmBackend::Http => cfg!(feature = "reqwest"),
            WasmBackend::Stomp => cfg!(feature = "tungstenite"),
        }
    }

    /// The concrete transport type the wasm newtype wraps.
    fn transport(self, paths: &Paths) -> syn::Path {
        match self {
            WasmBackend::Http => paths.plugin("client::ReqwestClient"),
            WasmBackend::Stomp => paths.plugin("client::StompClientTransport"),
        }
    }

    /// The constructor body building the wrapped client from the shared `Connection`.
    fn build_from_connection(self, client_ident: &Ident) -> TokenStream {
        match self {
            // HTTP is always ready; the STOMP socket may not be connected yet (fallible).
            WasmBackend::Http => {
                quote!(::core::result::Result::Ok(Self(#client_ident::new(connection.http()))))
            }
            WasmBackend::Stomp => {
                quote!(::core::result::Result::Ok(Self(#client_ident::new(connection.stomp()?))))
            }
        }
    }
}

/// Emits the **wasm** JavaScript binding's *struct* — a `#[wasm_bindgen]` newtype over the concrete
/// `{Client}<ReqwestClient>` (wasm-bindgen cannot export the generic form) plus its constructor.
/// Emitted **once** by `#[controller]` (like the generic client struct), so multiple `#[handlers]`
/// blocks — which contribute methods (see [`wasm_client_methods`]) — never re-declare it.
///
/// `#[doc(hidden)]` and exported under `js_name = "{Client}"`, so JS sees exactly `{Client}`; the
/// generic `{Client}<C>` is untouched (no native/wasm drift). Wasm-only, and only with the `reqwest`
/// fetch backend (the sole wasm HTTP transport). `wasm-bindgen`/`tsify` are the consuming crate's
/// direct wasm deps (their codegen hardcodes those crate paths, as is standard for a wasm crate).
pub fn wasm_client_struct(
    client_ident: &Ident,
    docs: &[syn::Attribute],
    backend: WasmBackend,
    paths: &Paths,
) -> TokenStream {
    if !backend.available() {
        return quote!();
    }

    let transport = backend.transport(paths);
    let connection = paths.plugin("client::Connection");
    let build = backend.build_from_connection(client_ident);
    let js_name = client_ident.to_string();
    let wrapper = wasm_wrapper_ident(client_ident);

    // The only doc comments on the generated wasm items are the controller type's own `#[doc]`s
    // (`docs`) — wasm-bindgen turns them into the exported class's TypeScript JSDoc, so the client is
    // documented exactly like its controller. No framework prose is emitted (it would leak into the
    // user's `.d.ts`). `#[doc(hidden)]` guards the internal Rust newtype from ever appearing in
    // rust docs no matter what docs.rs does; wasm-bindgen ignores it, so the JSDoc still emits.
    quote! {
        #(#docs)*
        #[cfg(target_family = "wasm")]
        #[doc(hidden)]
        #[::wasm_bindgen::prelude::wasm_bindgen(js_name = #js_name)]
        pub struct #wrapper(#client_ident<#transport>);

        // A separate `impl` from the method blocks (below, per `#[handlers]`); `js_class` ties every
        // block to the renamed exported class. wasm-bindgen composes multiple impls for one class.
        #[cfg(target_family = "wasm")]
        #[::wasm_bindgen::prelude::wasm_bindgen(js_class = #js_name)]
        impl #wrapper {
            /// Builds the client from a shared [`Connection`], so every client reuses its one
            /// underlying transport (the HTTP pool + cookies, or the STOMP socket).
            #[::wasm_bindgen::prelude::wasm_bindgen(constructor)]
            pub fn new(
                connection: &#connection,
            ) -> ::core::result::Result<#wrapper, ::wasm_bindgen::JsError> {
                #build
            }
        }
    }
}

/// Emits the wasm binding's *methods* — one `js_class`-tagged `impl` on the wrapper forwarding each
/// unary method with typed `Ts<T>` (or, by default, `into_wasm_abi`) arguments/returns, so wasm-pack
/// generates real TypeScript types rather than `any`. Emitted **per `#[handlers]` block** (like the
/// generic client methods), so several blocks compose onto the one struct without duplication.
fn wasm_client_methods(
    client_ident: &Ident,
    methods: &[ClientMethod],
    backend: WasmBackend,
    paths: &Paths,
) -> TokenStream {
    let js_name = client_ident.to_string();
    let wrapper = wasm_wrapper_ident(client_ident);
    let headers_ty = paths.plugin("client::RequestHeaders");

    // The wasm ABI flavour (see `#[dto]`): with `wasm-ts` the payloads are wrapped in `Ts<T>` and
    // converted with `.to_rust()`/`.into_ts()`; by default the `into_wasm_abi`/`from_wasm_abi` derives
    // make the types usable directly as `#[wasm_bindgen]` arguments/returns (no wrapper, no convert).
    let ts = cfg!(feature = "wasm-ts");

    let fns = methods
        .iter()
        .filter(|m| m.capability == Capability::Unary)
        .map(|m| {
            let ident = &m.ident;

            // Each path/query parameter, typed for TypeScript. `Ts<T>` (new ABI) needs a `.to_rust()`
            // before the call; the default ABI passes the value straight through.
            let extra_params = m.extra_args.iter().map(|(name, ty)| {
                if ts {
                    quote!(, #name: ::tsify::Ts<#ty>)
                } else {
                    quote!(, #name: #ty)
                }
            });
            let extra_prep = m.extra_args.iter().map(|(name, _)| {
                if ts {
                    quote!(let #name = #name.to_rust().map_err(::wasm_bindgen::JsError::from)?;)
                } else {
                    quote!()
                }
            });

            let mut call_args: Vec<TokenStream> =
                m.extra_args.iter().map(|(name, _)| quote!(#name)).collect();

            let (body_param, body_prep) = match &m.request {
                Some(req) => {
                    call_args.push(quote!(__request));

                    if ts {
                        (
                            quote!(, body: ::tsify::Ts<#req>),
                            quote!(let __request = body.to_rust().map_err(::wasm_bindgen::JsError::from)?;),
                        )
                    } else {
                        (quote!(, __request: #req), quote!())
                    }
                }

                None => (quote!(), quote!()),
            };

            // An HTTP method takes an optional per-call `Headers` (a browser builds it with
            // `new Headers().set(..)`) and routes through the generic `{method}_with_headers`; the
            // parameter is `Option`, so JS may omit it. A STOMP `SEND` has no HTTP headers.
            let (header_param, header_prep, target) = match backend {
                WasmBackend::Http => {
                    call_args.push(quote!(__headers));

                    (
                        quote!(, headers: ::core::option::Option<#headers_ty>),
                        quote!(let __headers = headers.map(#headers_ty::into_inner);),
                        format_ident!("{}_with_headers", ident),
                    )
                }

                WasmBackend::Stomp => (quote!(), quote!(), ident.clone()),
            };

            let response = &m.response;

            // The reply shape depends on the transport: an HTTP unary yields an `HttpResponse`
            // envelope whose body is the typed payload; a STOMP `SEND` yields unit (`void` in JS).
            let (ret, ret_expr) = match backend {
                WasmBackend::Http if ts => (
                    quote!(::tsify::Ts<#response>),
                    quote!(__response.into_body().into_ts().map_err(::wasm_bindgen::JsError::from)?),
                ),
                WasmBackend::Http => (quote!(#response), quote!(__response.into_body())),
                WasmBackend::Stomp => (quote!(()), quote!(__response)),
            };

            quote! {
                pub async fn #ident(
                    &self #(#extra_params)* #body_param #header_param
                ) -> ::core::result::Result<#ret, ::wasm_bindgen::JsError> {
                    #(#extra_prep)*
                    #body_prep
                    #header_prep

                    let __response = self
                        .0
                        .#target(#(#call_args),*)
                        .await
                        .map_err(|e| ::wasm_bindgen::JsError::new(
                            &::std::string::ToString::to_string(&e),
                        ))?;

                    ::core::result::Result::Ok(#ret_expr)
                }
            }
        });

    quote! {
        #[cfg(target_family = "wasm")]
        #[::wasm_bindgen::prelude::wasm_bindgen(js_class = #js_name)]
        impl #wrapper {
            #(#fns)*
        }
    }
}

/// The extra client-side tokens the HTTP extension emits alongside a `#[handlers]` block's generated
/// client methods: the deduped `Dto` wire-type assertions (both targets) and, with the `reqwest`
/// (fetch) backend, the wasm binding's *method* impl (wasm-only, self-gated). The wrapper struct is
/// emitted separately by `#[controller]` ([`wasm_client_struct`]). This is the whole of the axum
/// extension's [`extra_client_tokens`](overseerd_macros_core::ParseMethod::extra_client_tokens).
pub fn extra_client_tokens(
    client_ident: &Ident,
    methods: &[ClientMethod],
    header_methods: &[ClientMethod],
    wire_types: Vec<Type>,
    backend: Option<WasmBackend>,
    paths: &Paths,
) -> TokenStream {
    let assertions = dto_assertions(wire_types, paths);

    // The `{method}_with_headers` siblings, rendered with the same capability machinery as the base
    // methods (the framework core stays header-agnostic; header handling lives entirely here). They
    // go in their own `impl<C>` block — native Rust callers use them directly, and the wasm wrapper
    // calls them under the hood so a browser client's plain method can take optional headers. Emitted
    // on both targets (the wasm wrapper needs them) but only with the `client` feature.
    let with_headers = if cfg!(feature = "client") && !header_methods.is_empty() {
        let fns = header_methods
            .iter()
            .map(|m| overseerd_macros_core::client::client_method_tokens(m, paths));

        quote! {
            impl<C> #client_ident<C> {
                #(#fns)*
            }
        }
    } else {
        quote!()
    };

    // The wasm method impl (the struct comes from `#[controller]`). Only when the block has a wasm
    // backend (HTTP, or STOMP `SEND`) and that backend's transport is compiled in.
    let wasm_methods = match backend {
        Some(backend) if backend.available() && !methods.is_empty() => {
            wasm_client_methods(client_ident, methods, backend, paths)
        }

        _ => quote!(),
    };

    quote! {
        #assertions

        #with_headers

        #wasm_methods
    }
}

/// The client method's body **parameter type** (`request: <ty>`) for a classified body: the raw `T`
/// for `Json`/`Form`, a `Vec<u8>` for the raw byte bodies, or the `Multipart` builder. `None` for a
/// no-body route.
fn body_param_type(body: &Option<Body>, paths: &Paths) -> Option<Type> {
    let Body { kind, inner } = body.as_ref()?;

    let ty = match kind {
        BodyKind::Json | BodyKind::Form => inner
            .clone()
            .expect("a Json/Form body carries its payload type"),

        BodyKind::Bytes | BodyKind::RawForm => syn::parse_quote!(::std::vec::Vec<u8>),

        BodyKind::Multipart => {
            let multipart = paths.plugin("client::Multipart");

            syn::parse_quote!(#multipart)
        }
    };

    Some(ty)
}

/// The body param declaration (`, request: T`), the `HttpBody` wrapper type (drives `Encodes<B>`, the
/// envelope, and the content type), and the wrapped value expression (`wrapper(request)`) for a
/// classified body. The wrappers are client-owned (not axum's), so the body path carries no
/// dependency on the axum server framework and compiles for wasm; the method still takes the
/// ergonomic param, so the swap is invisible to callers. A no-body route encodes `()`.
fn body_parts(body: &Option<Body>, paths: &Paths) -> (TokenStream, TokenStream, TokenStream) {
    let Some(Body { kind, inner }) = body else {
        return (quote!(), quote!(()), quote!(()));
    };

    let (encode_ty, body_value) = match kind {
        BodyKind::Json => {
            let wrapper = paths.plugin("client::Json");

            (quote!(#wrapper<#inner>), quote!(#wrapper(request)))
        }

        BodyKind::Form => {
            let wrapper = paths.plugin("client::Form");

            (quote!(#wrapper<#inner>), quote!(#wrapper(request)))
        }

        BodyKind::Bytes => {
            let wrapper = paths.plugin("client::OctetStream");

            (quote!(#wrapper), quote!(#wrapper(request)))
        }

        BodyKind::RawForm => {
            let wrapper = paths.plugin("client::RawForm");

            (quote!(#wrapper), quote!(#wrapper(request)))
        }

        // `Multipart` is already an `HttpBody`, so it is its own wrapper — no re-wrap.
        BodyKind::Multipart => {
            let multipart = paths.plugin("client::Multipart");

            (quote!(#multipart), quote!(request))
        }
    };

    let param_ty = body_param_type(body, paths);

    (quote!(, request: #param_ty), encode_ty, body_value)
}

/// The query method **parameter** (name + type, appended to the leading path params) and the URI
/// **suffix** expression (`"?…"` or `""`) for a classified query. A typed `Query<T>` is URL-encoded
/// from the `Dto` `T`; a `RawQuery` takes an `Option<String>` and appends it verbatim. `None` when
/// the route has no query.
fn query_parts(
    query: &Option<QueryInput>,
    paths: &Paths,
) -> Option<((Ident, TokenStream), TokenStream)> {
    let name = format_ident!("query");

    let (param_ty, suffix) = match query.as_ref()? {
        QueryInput::Typed(ty) => {
            let encode_query = paths.plugin("client::encode_query");

            (
                quote!(#ty),
                quote! {{
                    let __q = #encode_query(&#name);

                    if __q.is_empty() {
                        ::std::string::String::new()
                    } else {
                        ::std::format!("?{}", __q)
                    }
                }},
            )
        }

        QueryInput::Raw => (
            quote!(::core::option::Option<::std::string::String>),
            quote! {
                match &#name {
                    ::core::option::Option::Some(__q) if !__q.is_empty() => {
                        ::std::format!("?{}", __q)
                    }

                    _ => ::std::string::String::new(),
                }
            },
        ),
    };

    Some(((name, param_ty), suffix))
}

/// Builds the `http::Request<B>` constructor expression for a route: verb + `BASE`-joined URI
/// (path params substituted, query appended) + the body's content type + the typed body value.
#[allow(clippy::too_many_arguments)]
fn request_builder(
    route: &RouteAttr,
    fmt: &str,
    subst: &[TokenStream],
    query_suffix: &TokenStream,
    encode_ty: &TokenStream,
    body_value: &TokenStream,
    per_call_headers: bool,
    paths: &Paths,
) -> TokenStream {
    let http = paths.plugin("http");
    let http_body = paths.plugin("client::HttpBody");
    let encode_path_segment = paths.plugin("client::encode_path_segment");
    let verb = format_ident!("{}", route.verb.to_string().to_uppercase());
    // The route base lives on the client struct (`impl {Controller}Client { const BASE }`), so the
    // URI is built from `Self::BASE` without depending on the server-only `Controller` trait.
    let base = quote!(Self::BASE);
    let subst = subst
        .iter()
        .map(|subst| quote!(#encode_path_segment(&#subst)));
    // The path template (holes filled) then the query suffix (`"?…"` or `""`).
    let uri =
        quote!(::std::format!("{}{}", ::std::format!(#fmt, #base #(, #subst)*), #query_suffix));

    // Fold the per-call `headers` argument (if this method takes one) over the built request; a
    // caller's header wins over the content type set above (`insert` replaces).
    let header_merge = if per_call_headers {
        quote! {
            if let ::core::option::Option::Some(__headers) = headers {
                for (__name, __value) in __headers.iter() {
                    __request.headers_mut().insert(__name.clone(), __value.clone());
                }
            }
        }
    } else {
        quote!()
    };

    quote! {{
        let mut __builder = #http::Request::builder()
            .method(#http::Method::#verb)
            .uri(#uri);

        if let ::core::option::Option::Some(__ct) = <#encode_ty as #http_body>::CONTENT_TYPE {
            __builder = __builder.header(#http::header::CONTENT_TYPE, __ct);
        }

        let mut __request = __builder
            .body(#body_value)
            .expect("client request is valid by construction");

        #header_merge

        __request
    }}
}

/// Builds the [`ClientMethod`] hint for a `streamed` route — a server-streaming method the
/// framework's `generate_client` emits from the **override** hints, returning
/// `impl Stream<Item = Result<T, ClientError>>` (the wire framing never appears in the
/// signature). `None` if the route is not `streamed` or its inputs/return are not classifiable.
///
/// The framing is read from the return wrapper (`Ndjson<..>` / `RawStream<..>` / any
/// `StreamDecode` impl), never hard-wired: the body decodes via `<Wrapper<()> as
/// StreamDecode<Item>>`. The override hints carry the HTTP-specific bound (`C: HttpStreaming +
/// Encodes<B>`), the `impl Stream` return, and the byte-stream-then-decode body; the framework
/// assembles the signature (args = path params + optional body, from the `ServerStreaming` arm).
pub fn build_stream_client_method(
    method_ident: &Ident,
    route: &RouteAttr,
    arg_types: &[&Type],
    wrapper_unit: TokenStream,
    item: Type,
    paths: &Paths,
) -> Option<ClientMethod> {
    let inputs = classify(arg_types)?;

    let (_req_param, encode_ty, body_value) = body_parts(&inputs.body, paths);
    let request_param = body_param_type(&inputs.body, paths);
    let (fmt, holes) = parse_template(&route.path.value());
    let path_plan = plan_path(&holes, inputs.path_ty)?;

    // The query param (after the path params) and the URI suffix it produces.
    let query_plan = query_parts(&inputs.query, paths);
    let query_suffix = query_plan
        .as_ref()
        .map(|(_, suffix)| suffix.clone())
        .unwrap_or_else(|| quote!(""));

    let mut extra_args = path_plan.args;

    if let Some((query_arg, _)) = query_plan {
        extra_args.push(query_arg);
    }

    let request = request_builder(
        route,
        &fmt,
        &path_plan.subst,
        &query_suffix,
        &encode_ty,
        &body_value,
        false,
        paths,
    );

    let client_error = paths.client("ClientError");
    let http = paths.plugin("http");
    let encodes = paths.client("Encodes");
    let http_streaming = paths.plugin("client::HttpStreaming");
    let stream_decode = paths.plugin("client::StreamDecode");
    let stream_trait = paths.plugin("__Stream");

    // The body param (raw `T`) is supplied via `request`; the path/query params via `extra_args`. The
    // `ServerStreaming` arm assembles `&self, <path/query params>, request: T` from those.
    let request = ClientMethodOverrideBody {
        bounds: quote!(C: #http_streaming + #encodes<#encode_ty>),
        // The item type mirrors the server's exactly (`T`, or a `Result<T, E>` the handler chose
        // to stream); the outer `Result` carries only pre-stream errors. Transport/frame-decode
        // failures end the stream (logged), never surfaced as items.
        ret: quote! {
            ::core::result::Result<
                impl #stream_trait<Item = #item>,
                #client_error<#http::StatusCode>,
            >
        },
        body: quote! {{
            let __request = #request;
            let __bytes = #http_streaming::open_stream(&self.0, __request).await?;

            ::core::result::Result::Ok(
                <#wrapper_unit as #stream_decode<#item>>::decode_stream(__bytes),
            )
        }},
    };

    Some(ClientMethod {
        ident: method_ident.clone(),
        path: String::new(),
        capability: Capability::ServerStreaming,
        request: request_param,
        encode_as: None,
        req_item: None,
        resp_item: None,
        response: item,
        error_ty: None,
        extra_args,
        request_envelope: None,
        request_builder: None,
        response_envelope: None,
        response_mapper: None,
        trailing_args: ::std::vec::Vec::new(),
        attrs: ::std::vec::Vec::new(),
        override_bounds: Some(request.bounds),
        override_ret: Some(request.ret),
        override_body: Some(request.body),
    })
}

/// The three override hints a streamed route fills (grouped for readability).
struct ClientMethodOverrideBody {
    bounds: TokenStream,
    ret: TokenStream,
    body: TokenStream,
}

/// Builds the [`ClientMethod`] hint for a **client-streaming** route (a `#[stream]` request body):
/// the client takes `input: impl Into<StreamArg<T>>` (reusing the agnostic `ClientStreaming`
/// args), frames it NDJSON, and sends it as a streamed body for a unary response. `item` is the
/// `#[stream]` parameter's stream item type; `arg_types` excludes that parameter (path params
/// only). `None` if a path/return is not classifiable, or a `Json`/`Form` body also appears (the
/// stream *is* the body).
pub fn build_client_stream_method(
    method_ident: &Ident,
    route: &RouteAttr,
    arg_types: &[&Type],
    item: Type,
    output: &ReturnType,
    paths: &Paths,
) -> Option<ClientMethod> {
    let inputs = classify(arg_types)?;

    if inputs.body.is_some() {
        return None;
    }

    let (fmt, holes) = parse_template(&route.path.value());
    let path_plan = plan_path(&holes, inputs.path_ty)?;
    let response = response_type(output);

    let http = paths.plugin("http");
    let ndjson = paths.plugin("Ndjson");
    let stream_encode = paths.plugin("StreamEncode");
    let encode_stream = paths.plugin("client::encode_stream");
    let http_client_streaming = paths.plugin("client::HttpClientStreaming");
    let http_response = paths.plugin("client::HttpResponse");
    let encode_path_segment = paths.plugin("client::encode_path_segment");
    let client_error = paths.client("ClientError");
    let decodes = paths.client("Decodes");
    let stream_arg = paths.client("StreamArg");

    // The query param (after the path params) and the URI suffix it produces.
    let query_plan = query_parts(&inputs.query, paths);
    let query_suffix = query_plan
        .as_ref()
        .map(|(_, suffix)| suffix.clone())
        .unwrap_or_else(|| quote!(""));

    let mut extra_args = path_plan.args;

    if let Some((query_arg, _)) = query_plan {
        extra_args.push(query_arg);
    }

    let verb = format_ident!("{}", route.verb.to_string().to_uppercase());
    // Built from the client struct's `Self::BASE` (see `request_builder`).
    let base = quote!(Self::BASE);
    let subst = &path_plan.subst;
    let subst = subst
        .iter()
        .map(|subst| quote!(#encode_path_segment(&#subst)));
    // The path template (holes filled) then the query suffix (`"?…"` or `""`).
    let uri =
        quote!(::std::format!("{}{}", ::std::format!(#fmt, #base #(, #subst)*), #query_suffix));

    let request = ClientMethodOverrideBody {
        bounds: quote!(C: #http_client_streaming + #decodes<#response>),
        ret: quote!(::core::result::Result<#http_response<#response>, #client_error<#http::StatusCode>>),
        body: quote! {{
            // The agnostic `StreamArg` carries the input stream; frame it NDJSON and send it as a
            // streamed request body.
            let __stream = ::core::convert::Into::<#stream_arg<#item>>::into(input).into_inner();
            let __body = #encode_stream::<#ndjson<()>, #item, _>(__stream);

            let __request = #http::Request::builder()
                .method(#http::Method::#verb)
                .uri(#uri)
                .header(
                    #http::header::CONTENT_TYPE,
                    <#ndjson<()> as #stream_encode<#item>>::CONTENT_TYPE,
                )
                .body(__body)
                .expect("client request is valid by construction");

            #http_client_streaming::send_stream(&self.0, __request).await
        }},
    };

    Some(ClientMethod {
        ident: method_ident.clone(),
        path: String::new(),
        capability: Capability::ClientStreaming,
        request: None,
        encode_as: None,
        // Drives the `ClientStreaming` arm's `input: impl Into<StreamArg<#item>>` argument.
        req_item: Some(item),
        resp_item: None,
        response,
        error_ty: None,
        extra_args,
        request_envelope: None,
        request_builder: None,
        response_envelope: None,
        response_mapper: None,
        trailing_args: ::std::vec::Vec::new(),
        attrs: ::std::vec::Vec::new(),
        override_bounds: Some(request.bounds),
        override_ret: Some(request.ret),
        override_body: Some(request.body),
    })
}

/// From a streamed return type `Wrapper<impl Stream<Item = T>>` (or `Wrapper<ConcreteStream>`),
/// recovers `(Wrapper<()>, Item)`: the framing wrapper with its stream parameter erased (the
/// `StreamDecode` marker) and the decoded item type. The item is read from a `Stream<Item = T>`
/// `impl Trait` binding, or projected as `<ConcreteStream as Stream>::Item`.
/// How the macro wraps a bare `impl Stream<..>` return server-side (it is not `IntoResponse`
/// itself). The inferred framing of the **shorthand registry**.
pub(crate) enum ServerWrap {
    /// `T != u8` → `Ndjson(stream)`.
    Ndjson,
    /// `T == u8` → `RawStream(chunk_u8(stream))`.
    RawU8,
}

/// A classified server-streaming return.
pub(crate) struct StreamReturn {
    /// How to wrap the handler's return server-side; `None` when the return is already
    /// `IntoResponse` (an explicit wrapper, or a flagged opaque type).
    pub server_wrap: Option<ServerWrap>,
    /// Whether the stream sits inside an outer `Result<.., E>` (a pre-stream failure). The server
    /// wrap then maps over the `Result` rather than wrapping it.
    pub in_result: bool,
    /// The client decode `(Wrapper<()>, item)`, or `None` for a flagged-opaque return — server
    /// passes it through but the macro can't generate a client method for an unknown wire format.
    pub client: Option<(TokenStream, Type)>,
}

/// The streaming **shorthand registry** — the single, extensible place that maps a
/// server-streaming return to its wire framing. Built-in shorthands (peeling an outer
/// `Result<.., E>` pre-stream-failure first):
///
/// - `Ndjson<S>`                  → NDJSON, client item `S::Item`   (already `IntoResponse`)
/// - `RawStream<S>`               → raw bytes, client item `Bytes`  (already `IntoResponse`)
/// - bare `impl Stream<Item = u8>`      → raw bytes (macro wraps `RawStream`)
/// - bare `impl Stream<Item = T>` (T≠u8) → NDJSON   (macro wraps `Ndjson`)
///
/// Add a built-in by extending the matches here. Anything else is server-streaming **only** with
/// the `streamed` route flag — the server passes it through (the user's own `IntoResponse` wire
/// format) and no client method is generated.
pub(crate) fn classify_stream_return(
    output: &ReturnType,
    streamed: bool,
    paths: &Paths,
) -> Option<StreamReturn> {
    let ReturnType::Type(_, ty) = output else {
        return None;
    };

    // Peel an outer `Result<Inner, E>` (a pre-stream failure); the client models `E` as the
    // outer `ClientError`, so only the inner stream shape matters here.
    let (inner, in_result) = match first_type_arg(ty, "Result") {
        Some(ok) => (ok, true),
        None => ((**ty).clone(), false),
    };

    // A known framing wrapper — already `IntoResponse`, so no server wrap; the client decodes via
    // `<Wrapper<()> as StreamDecode<item>>`.
    if let Type::Path(type_path) = &inner
        && let Some(segment) = type_path.path.segments.last()
        && matches!(segment.ident.to_string().as_str(), "Ndjson" | "RawStream")
        && let PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(GenericArgument::Type(stream_ty)) = args.args.first()
    {
        let mut bare = type_path.clone();
        bare.path.segments.last_mut()?.arguments = PathArguments::None;

        return Some(StreamReturn {
            server_wrap: None,
            in_result,
            client: Some((quote!(#bare<()>), stream_item(stream_ty, paths)?)),
        });
    }

    // A bare `impl Stream<Item = T>` — the macro wraps it server-side: raw bytes when `T = u8`,
    // NDJSON otherwise. The item is read *raw* (a `Result<T, E>` item stays intact, so the client
    // mirrors it) — unlike RPC's fallible-stream peeling.
    if let Type::ImplTrait(impl_trait) = &inner
        && let Some(item) = stream_item_binding(impl_trait)
    {
        let ndjson = paths.plugin("Ndjson");
        let raw = paths.plugin("RawStream");

        if type_name(&item).is_some_and(|name| name == "u8") {
            let bytes = paths.plugin("bytes::Bytes");

            return Some(StreamReturn {
                server_wrap: Some(ServerWrap::RawU8),
                in_result,
                client: Some((quote!(#raw<()>), syn::parse_quote!(#bytes))),
            });
        }

        return Some(StreamReturn {
            server_wrap: Some(ServerWrap::Ndjson),
            in_result,
            client: Some((quote!(#ndjson<()>), item)),
        });
    }

    // An opaque/custom return is server-streaming only when explicitly flagged; the server passes
    // it through, and no client method is generated (the wire format is the user's own).
    streamed.then_some(StreamReturn {
        server_wrap: None,
        in_result,
        client: None,
    })
}

/// The item type `T` of a stream type — `<S as Stream>::Item`. Read from a `Stream<Item = T>`
/// `impl Trait` binding, or projected on a concrete stream type. Shared by the server-streaming
/// return analysis and the client-streaming `#[stream]` parameter analysis.
pub(crate) fn stream_item(ty: &Type, paths: &Paths) -> Option<Type> {
    match ty {
        Type::ImplTrait(impl_trait) => stream_item_binding(impl_trait),

        concrete => {
            let stream = paths.plugin("__Stream");

            Some(syn::parse_quote!(<#concrete as #stream>::Item))
        }
    }
}

/// The `T` of a `Stream<Item = T>` bound within an `impl Trait`.
fn stream_item_binding(impl_trait: &syn::TypeImplTrait) -> Option<Type> {
    for bound in &impl_trait.bounds {
        let TypeParamBound::Trait(trait_bound) = bound else {
            continue;
        };

        let segment = trait_bound.path.segments.last()?;

        if segment.ident != "Stream" {
            continue;
        }

        let PathArguments::AngleBracketed(args) = &segment.arguments else {
            continue;
        };

        for arg in &args.args {
            if let GenericArgument::AssocType(assoc) = arg
                && assoc.ident == "Item"
            {
                return Some(assoc.ty.clone());
            }
        }
    }

    None
}

/// axum extractors that consume wire data the generated client still cannot encode. A route using one
/// opts out of client generation (see [`classify`]) rather than get a method that silently omits that
/// data. `Request` (and its alias `RawRequest`) take the *entire* request — headers and an opaque
/// body — so there is no typed shape to reconstruct on the client. The recognized wire inputs
/// (`Path`/`Query`/`Json`/`Form`/`Bytes`/`RawForm`/`Multipart`) are handled in [`classify`]; server-only
/// context (`Inject`/`State`/… and custom `FromRequestParts` guards) carries nothing over the wire and
/// is dropped, leaving the method intact.
const UNSUPPORTED_WIRE: &[&str] = &["Request", "RawRequest"];

/// Above this many path holes, the params collapse into a single tuple argument rather than one
/// named argument each — keeping a long route's client signature compact.
const MAX_NAMED_PATH_PARAMS: usize = 3;

/// Which [`HttpBody`](../overseerd_axum/client/trait.HttpBody.html) wrapper a route's body uses (the
/// wrapper owns the content type + wire encoding). `Json`/`Form` wrap a serde payload `T`; `Bytes`
/// and `RawForm` wrap a raw `Vec<u8>`; `Multipart` is the `Multipart` builder, already an `HttpBody`.
enum BodyKind {
    Json,
    Form,
    Bytes,
    RawForm,
    Multipart,
}

/// Builds the client methods for one unary route — the clean `method` and its `method_with_headers`
/// sibling ([`UnaryMethods`]) — or `None` if an argument is not a recognized client input or a
/// droppable server-only extractor (the route then gets no client method).
pub fn build_client_method(
    method_ident: &Ident,
    route: &RouteAttr,
    arg_types: &[&Type],
    output: &ReturnType,
    paths: &Paths,
) -> Option<UnaryMethods> {
    // A `streamed` route is server-streaming — see `build_stream_client_method`. The unary form
    // does not apply.
    if route.streamed {
        return None;
    }

    let inputs = classify(arg_types)?;
    let (fmt, holes) = parse_template(&route.path.value());
    let path_plan = plan_path(&holes, inputs.path_ty)?;

    Some(assemble(
        method_ident,
        route,
        fmt,
        path_plan,
        inputs.query,
        inputs.body,
        output,
        paths,
    ))
}

/// The path parameters of a route: the method's leading args and the per-hole substitution
/// expressions (in route order) feeding the URI `format!`.
struct PathPlan {
    args: Vec<(Ident, TokenStream)>,
    subst: Vec<TokenStream>,
}

/// Maps the route's holes to the `Path<T>` type, producing dedicated named params (named after
/// the holes) up to [`MAX_NAMED_PATH_PARAMS`], or a single tuple param beyond that. Returns
/// `None` on a hole/type-arity mismatch (a malformed handler — skip its client method).
fn plan_path(holes: &[String], path_ty: Option<Type>) -> Option<PathPlan> {
    if holes.is_empty() {
        return Some(PathPlan {
            args: Vec::new(),
            subst: Vec::new(),
        });
    }

    let path_ty = path_ty?;

    // One hole: the whole `Path<T>` type is that param, named after the hole.
    if holes.len() == 1 {
        let name = hole_ident(&holes[0], 0);

        return Some(PathPlan {
            args: vec![(name.clone(), quote!(#path_ty))],
            subst: vec![quote!(#name)],
        });
    }

    // Many holes: `Path<(A, B, ..)>` — the tuple arity must match.
    let elems = tuple_elems(&path_ty)?;

    if elems.len() != holes.len() {
        return None;
    }

    if holes.len() <= MAX_NAMED_PATH_PARAMS {
        // Dedicated params, named after the holes, typed by the tuple elements.
        let args = holes
            .iter()
            .zip(&elems)
            .enumerate()
            .map(|(i, (hole, ty))| (hole_ident(hole, i), quote!(#ty)))
            .collect::<Vec<_>>();
        let subst = args.iter().map(|(name, _)| quote!(#name)).collect();

        return Some(PathPlan { args, subst });
    }

    // Too many: a single tuple param, substituted by index.
    let subst = (0..holes.len())
        .map(syn::Index::from)
        .map(|idx| quote!(path.#idx))
        .collect();

    Some(PathPlan {
        args: vec![(format_ident!("path"), quote!(#path_ty))],
        subst,
    })
}

/// Assembles the hint once the args are classified.
/// The two forms of a generated unary method: the clean `method` and its `method_with_headers`
/// sibling (identical but for a trailing per-call `headers` argument the body folds into the request).
pub struct UnaryMethods {
    pub base: ClientMethod,
    pub with_headers: ClientMethod,
}

#[allow(clippy::too_many_arguments)]
fn assemble(
    method_ident: &Ident,
    route: &RouteAttr,
    fmt: String,
    path_plan: PathPlan,
    query: Option<QueryInput>,
    body: Option<Body>,
    output: &ReturnType,
    paths: &Paths,
) -> UnaryMethods {
    let http = paths.plugin("http");
    let http_response = paths.plugin("client::HttpResponse");

    // The decoded response body: peel a `Json<T>` return to `T`, else the bare return type.
    let response = response_type(output);

    // The body: the raw `T` (or `Vec<u8>` / `Multipart`) is the param, but the wire body is its
    // `HttpBody` wrapper — which drives the `Encodes<B>` bound, the envelope, and the content type.
    let request = body_param_type(&body, paths);
    let (_body_param, encode_ty, body_value) = body_parts(&body, paths);
    let encode_as = request.as_ref().map(|_| encode_ty.clone());

    // The query param (folded in after the path params) and the URI suffix it produces.
    let query_plan = query_parts(&query, paths);
    let query_suffix = query_plan
        .as_ref()
        .map(|(_, suffix)| suffix.clone())
        .unwrap_or_else(|| quote!(""));

    let mut extra_args = path_plan.args;

    if let Some((query_arg, _)) = query_plan {
        extra_args.push(query_arg);
    }

    // The single request-building path lives on `_with_headers`: it folds the per-call `headers`
    // argument over the request. Header/URI are valid by construction, so it cannot fail here.
    let header_builder = request_builder(
        route,
        &fmt,
        &path_plan.subst,
        &query_suffix,
        &encode_ty,
        &body_value,
        true,
        paths,
    );

    let request_envelope = Some(quote!(#http::Request<#encode_ty>));
    let response_envelope = Some(quote!(#http_response<#response>));

    // The argument names the plain method forwards into `_with_headers` (path/query params, then the
    // body), followed by `None` for the headers — so the plain call is `_with_headers(.., None)`.
    let with_ident = format_ident!("{}_with_headers", method_ident);
    let mut forward_args: Vec<TokenStream> =
        extra_args.iter().map(|(name, _)| quote!(#name)).collect();

    if request.is_some() {
        forward_args.push(quote!(request));
    }

    // `_with_headers` — the real method: the full request-building body plus a trailing
    // `headers: Option<HeaderMap>` argument.
    let with_headers = ClientMethod {
        ident: with_ident.clone(),
        // Empty: the method and the full URI live in the `http::Request` envelope the
        // `request_builder` constructs, so the capability's `path` arg carries nothing for HTTP
        // (it exists for RPC's `"Service.method"` routing). The transport reads `request.uri()`.
        path: String::new(),
        capability: Capability::Unary,
        request: request.clone(),
        encode_as: encode_as.clone(),
        req_item: None,
        resp_item: None,
        response: response.clone(),
        error_ty: None,
        extra_args: extra_args.clone(),
        request_envelope: request_envelope.clone(),
        request_builder: Some(header_builder),
        response_envelope: response_envelope.clone(),
        response_mapper: None,
        trailing_args: vec![(
            format_ident!("headers"),
            quote!(::core::option::Option<#http::HeaderMap>),
        )],
        attrs: Vec::new(),
        override_bounds: None,
        override_ret: None,
        override_body: None,
    };

    // The plain method: the same signature minus the headers argument, forwarding to `_with_headers`
    // with no per-call headers. It carries no request builder of its own, so there is a single
    // request-building path and no drift; `#[inline(always)]` keeps the extra hop free.
    let base = ClientMethod {
        ident: method_ident.clone(),
        path: String::new(),
        capability: Capability::Unary,
        request,
        encode_as,
        req_item: None,
        resp_item: None,
        response,
        error_ty: None,
        extra_args,
        request_envelope,
        request_builder: None,
        response_envelope,
        response_mapper: None,
        trailing_args: Vec::new(),
        attrs: vec![quote!(#[inline(always)])],
        override_bounds: None,
        override_ret: None,
        override_body: Some(quote! {
            self.#with_ident(#(#forward_args,)* ::core::option::Option::None).await
        }),
    };

    UnaryMethods { base, with_headers }
}

/// The decoded response body type: the `T` of a `Json<T>` return (the common case), or the bare
/// return type, or `()` for no return.
pub(crate) fn response_type(output: &ReturnType) -> Type {
    match output {
        ReturnType::Type(_, ty) => {
            let inner = first_type_arg(ty, "Result").unwrap_or_else(|| (**ty).clone());

            first_type_arg(&inner, "Json").unwrap_or(inner)
        }

        ReturnType::Default => syn::parse_quote!(()),
    }
}

/// The element types of a tuple type, or `None` if it is not a tuple.
fn tuple_elems(ty: &Type) -> Option<Vec<Type>> {
    match ty {
        Type::Tuple(tuple) => Some(tuple.elems.iter().cloned().collect()),

        _ => None,
    }
}

/// A valid parameter identifier for a route hole (stripping a leading `*` wildcard marker),
/// falling back to `path{i}` when the hole is not a plain identifier.
fn hole_ident(hole: &str, index: usize) -> Ident {
    let name = hole.trim_start_matches('*');

    if !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !name.chars().next().is_some_and(|c| c.is_numeric())
        && syn::parse_str::<Ident>(name).is_ok()
    {
        format_ident!("{}", name)
    } else {
        format_ident!("path{}", index)
    }
}

/// Turns a route template into a `format!` string and the ordered hole names, following
/// **matchit 0.8's** grammar (the matcher axum uses) so the client agrees with how the server
/// route is matched — `{name}` and `{*catch_all}` are params, and `{{` / `}}` are escaped
/// literal braces. The same template literal feeds both `axum::Router::route(..)` and this, so
/// hole *positions* cannot drift; only the grammar must match, which these rules and the tests
/// below pin. matchit exposes no public template parser to reuse directly.
///
/// Each param hole becomes a positional `{}`; an escaped `{{`/`}}` becomes a literal `{{`/`}}`
/// in the `format!` string (which renders as a single brace). The leading `{}` for `BASE` is
/// prepended, so the returned string starts with `{}` then the templated remainder.
fn parse_template(template: &str) -> (String, Vec<String>) {
    let mut out = String::from("{}");
    let mut holes = Vec::new();
    let mut chars = template.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            // `{{` — an escaped literal `{`. Emit a literal brace (doubled for `format!`).
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push_str("{{");
            }

            // `{name}` / `{*catch_all}` — a parameter hole.
            '{' => {
                let mut name = String::new();

                for inner in chars.by_ref() {
                    if inner == '}' {
                        break;
                    }

                    name.push(inner);
                }

                out.push_str("{}");
                holes.push(name);
            }

            // `}}` — an escaped literal `}`.
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push_str("}}");
            }

            // A bare `}` is invalid in matchit (and rejected by axum at route registration); emit
            // it escaped so the `format!` string stays valid rather than failing to compile here.
            '}' => out.push_str("}}"),

            _ => out.push(c),
        }
    }

    (out, holes)
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::{hole_ident, parse_template, response_type};

    /// The `format!` string starts with `{}` for `BASE`, each param hole is a positional `{}`,
    /// and the hole names are recovered in order — matching matchit 0.8's `{name}` syntax.
    #[test]
    fn params_become_positional_placeholders() {
        let (fmt, holes) = parse_template("/users/{id}/posts/{slug}");

        assert_eq!(fmt, "{}/users/{}/posts/{}");
        assert_eq!(holes, vec!["id".to_string(), "slug".to_string()]);
    }

    /// A no-param route has no holes; only the `BASE` placeholder leads.
    #[test]
    fn static_route_has_no_holes() {
        let (fmt, holes) = parse_template("/health");

        assert_eq!(fmt, "{}/health");
        assert!(holes.is_empty());
    }

    /// A `{*catch_all}` is one hole (named with its `*`, stripped later for the param ident).
    #[test]
    fn catch_all_is_one_hole() {
        let (fmt, holes) = parse_template("/files/{*path}");

        assert_eq!(fmt, "{}/files/{}");
        assert_eq!(holes, vec!["*path".to_string()]);
    }

    /// Escaped braces (`{{`/`}}`) are literal, not holes, and survive as literal braces in the
    /// `format!` string.
    #[test]
    fn escaped_braces_are_literal() {
        let (fmt, holes) = parse_template("/lit/{{x}}/{id}");

        assert_eq!(fmt, "{}/lit/{{x}}/{}");
        assert_eq!(holes, vec!["id".to_string()]);
    }

    #[test]
    fn keyword_holes_fall_back_to_generated_param_name() {
        assert_eq!(hole_ident("type", 0).to_string(), "path0");
        assert_eq!(hole_ident("self", 1).to_string(), "path1");
    }

    #[test]
    fn response_type_peels_result_then_json() {
        let output = syn::parse_quote!(-> Result<Json<User>, HandlerError>);
        let ty = response_type(&output);

        assert_eq!(quote!(#ty).to_string(), "User");
    }
}
