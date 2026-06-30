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
//! A route whose args are not all classifiable yields `None` — the server route still
//! registers, it simply gets no generated client method (rather than a silently wrong one).

use overseerd_macros_core::attr::{first_type_arg, type_name};
use overseerd_macros_core::client::{Capability, ClientMethod};
use overseerd_macros_core::paths::Paths;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{GenericArgument, Ident, PathArguments, ReturnType, Type, TypeParamBound};

use crate::route::RouteAttr;

/// The classified client inputs of a route: an optional `Path` parameter type and an optional
/// `(wrapper, inner)` body. Shared by the unary and streaming method builders.
struct Inputs {
    path_ty: Option<Type>,
    body: Option<(BodyKind, Type)>,
}

/// Classifies a route's handler arguments into client inputs, dropping server-only extractors.
/// `None` means an argument is neither a recognized client input nor a droppable server-only
/// extractor — the route then gets no generated client method.
fn classify(arg_types: &[&Type]) -> Option<Inputs> {
    let mut path_ty: Option<Type> = None;
    let mut body: Option<(BodyKind, Type)> = None;

    for ty in arg_types {
        if let Some(inner) = first_type_arg(ty, "Path") {
            if path_ty.is_some() {
                return None;
            }

            path_ty = Some(inner);

            continue;
        }

        if let Some(inner) = first_type_arg(ty, "Json") {
            if body.is_some() {
                return None;
            }

            body = Some((BodyKind::Json, inner));

            continue;
        }

        if let Some(inner) = first_type_arg(ty, "Form") {
            if body.is_some() {
                return None;
            }

            body = Some((BodyKind::Form, inner));

            continue;
        }

        match type_name(ty).map(Ident::to_string).as_deref() {
            Some(name) if SERVER_ONLY.contains(&name) => continue,

            _ => return None,
        }
    }

    Some(Inputs { path_ty, body })
}

/// The body wrapper path (`Json`/`Form`) for the wire body type and the `body: T` value.
fn body_parts(
    body: &Option<(BodyKind, Type)>,
    paths: &Paths,
) -> (TokenStream, TokenStream, TokenStream) {
    match body {
        Some((kind, inner)) => {
            let wrapper = match kind {
                BodyKind::Json => paths.plugin("axum::Json"),
                BodyKind::Form => paths.plugin("axum::extract::Form"),
            };

            (
                quote!(, request: #inner),
                quote!(#wrapper<#inner>),
                quote!(#wrapper(request)),
            )
        }

        None => (quote!(), quote!(()), quote!(())),
    }
}

/// Builds the `http::Request<B>` constructor expression for a route: verb + `BASE`-joined URI
/// (path params substituted) + the body's content type + the typed body value.
fn request_builder(
    controller: &Ident,
    route: &RouteAttr,
    fmt: &str,
    subst: &[TokenStream],
    encode_ty: &TokenStream,
    body_value: &TokenStream,
    paths: &Paths,
) -> TokenStream {
    let http = paths.plugin("http");
    let http_body = paths.plugin("client::HttpBody");
    let controller_trait = paths.plugin("Controller");
    let verb = format_ident!("{}", route.verb.to_string().to_uppercase());
    let base = quote!(<#controller as #controller_trait>::BASE);
    let uri = quote!(::std::format!(#fmt, #base #(, #subst)*));

    quote! {{
        let mut __builder = #http::Request::builder()
            .method(#http::Method::#verb)
            .uri(#uri);

        if let ::core::option::Option::Some(__ct) = <#encode_ty as #http_body>::CONTENT_TYPE {
            __builder = __builder.header(#http::header::CONTENT_TYPE, __ct);
        }

        __builder
            .body(#body_value)
            .expect("client request is valid by construction")
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
    controller: &Ident,
    method_ident: &Ident,
    route: &RouteAttr,
    arg_types: &[&Type],
    output: &ReturnType,
    paths: &Paths,
) -> Option<ClientMethod> {
    if !route.streamed {
        return None;
    }

    let inputs = classify(arg_types)?;
    let (wrapper_unit, item) = stream_wrapper_and_item(output, paths)?;

    let (_req_param, encode_ty, body_value) = body_parts(&inputs.body, paths);
    let (fmt, holes) = parse_template(&route.path.value());
    let path_plan = plan_path(&holes, inputs.path_ty)?;
    let request = request_builder(
        controller,
        route,
        &fmt,
        &path_plan.subst,
        &encode_ty,
        &body_value,
        paths,
    );

    let client_error = paths.client("ClientError");
    let encodes = paths.client("Encodes");
    let http_streaming = paths.plugin("client::HttpStreaming");
    let stream_decode = paths.plugin("client::StreamDecode");
    let stream_trait = paths.plugin("__Stream");

    // The body param (raw `T`) is supplied via `request`; the path params via `extra_args`. The
    // `ServerStreaming` arm assembles `&self, <path params>, request: T` from those.
    let request = ClientMethodOverrideBody {
        bounds: quote!(C: #http_streaming + #encodes<#encode_ty>),
        // The item type mirrors the server's exactly (`T`, or a `Result<T, E>` the handler chose
        // to stream); the outer `Result` carries only pre-stream errors. Transport/frame-decode
        // failures end the stream (logged), never surfaced as items.
        ret: quote! {
            ::core::result::Result<
                impl #stream_trait<Item = #item>,
                #client_error,
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
        request: inputs.body.map(|(_, inner)| inner),
        encode_as: None,
        req_item: None,
        resp_item: None,
        response: item,
        error_ty: None,
        extra_args: path_plan.args,
        request_envelope: None,
        request_builder: None,
        response_envelope: None,
        response_mapper: None,
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

/// From a streamed return type `Wrapper<impl Stream<Item = T>>` (or `Wrapper<ConcreteStream>`),
/// recovers `(Wrapper<()>, Item)`: the framing wrapper with its stream parameter erased (the
/// `StreamDecode` marker) and the decoded item type. The item is read from a `Stream<Item = T>`
/// `impl Trait` binding, or projected as `<ConcreteStream as Stream>::Item`.
fn stream_wrapper_and_item(output: &ReturnType, paths: &Paths) -> Option<(TokenStream, Type)> {
    let ReturnType::Type(_, ty) = output else {
        return None;
    };

    // A handler may declare a pre-stream failure as `Result<Wrapper<Stream>, E>` — peel that
    // outer `Result` to the wrapper. The server's `E` is a pre-stream error, which the client
    // already models as the outer `ClientError`; only the wrapper + item shape matters here.
    let wrapper = first_type_arg(ty, "Result").unwrap_or_else(|| (**ty).clone());
    let Type::Path(type_path) = &wrapper else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    let GenericArgument::Type(inner) = args.args.first()? else {
        return None;
    };

    // The wrapper with its stream parameter erased: `Ndjson<..>` → `Ndjson<()>`.
    let mut bare = type_path.clone();
    bare.path.segments.last_mut()?.arguments = PathArguments::None;
    let wrapper_unit = quote!(#bare<()>);

    let item = match inner {
        Type::ImplTrait(impl_trait) => stream_item_binding(impl_trait)?,

        concrete => {
            let stream = paths.plugin("__Stream");

            syn::parse_quote!(<#concrete as #stream>::Item)
        }
    };

    Some((wrapper_unit, item))
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

/// Server-only extractors: resolved on the server, never sent by the client, so they are
/// dropped from the client signature (the method is still emitted).
const SERVER_ONLY: &[&str] = &["Inject", "State", "Extension", "ConnectInfo"];

/// Above this many path holes, the params collapse into a single tuple argument rather than one
/// named argument each — keeping a long route's client signature compact.
const MAX_NAMED_PATH_PARAMS: usize = 3;

/// Which body wrapper a `Json`/`Form` argument uses (the `HttpBody` that owns its content type).
enum BodyKind {
    Json,
    Form,
}

/// Builds the [`ClientMethod`] hint for one route, or `None` if an argument is not a recognized
/// client input or droppable server-only extractor (the route then gets no client method).
pub fn build_client_method(
    controller: &Ident,
    method_ident: &Ident,
    route: &RouteAttr,
    arg_types: &[&Type],
    output: &ReturnType,
    paths: &Paths,
) -> Option<ClientMethod> {
    // A `streamed` route is server-streaming — see `build_stream_client_method`. The unary form
    // does not apply.
    if route.streamed {
        return None;
    }

    let inputs = classify(arg_types)?;
    let (fmt, holes) = parse_template(&route.path.value());
    let path_plan = plan_path(&holes, inputs.path_ty)?;

    Some(assemble(
        controller,
        method_ident,
        route,
        fmt,
        path_plan,
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
#[allow(clippy::too_many_arguments)]
fn assemble(
    controller: &Ident,
    method_ident: &Ident,
    route: &RouteAttr,
    fmt: String,
    path_plan: PathPlan,
    body: Option<(BodyKind, Type)>,
    output: &ReturnType,
    paths: &Paths,
) -> ClientMethod {
    let http = paths.plugin("http");
    let http_body = paths.plugin("client::HttpBody");
    let http_response = paths.plugin("client::HttpResponse");
    let controller_trait = paths.plugin("Controller");

    // The decoded response body: peel a `Json<T>` return to `T`, else the bare return type.
    let response = response_type(output);

    // The URI: `<Controller>::BASE` then the route template with each `{name}` hole replaced by
    // a positional `{}`, filled from the path params. One `http::Method::<VERB>` selects the verb.
    let verb = format_ident!("{}", route.verb.to_string().to_uppercase());
    let base = quote!(<#controller as #controller_trait>::BASE);
    let subst = &path_plan.subst;
    let uri = quote!(::std::format!(#fmt, #base #(, #subst)*));

    // The body: the raw `T` is the param, but the wire body is its `HttpBody` wrapper
    // (`Json<T>`/`Form<T>`) — which drives the `Encodes<B>` bound, the envelope, and the
    // content type. A no-body route uses `()`.
    let (request, encode_as, body_value, body_ty) = match body {
        Some((kind, inner)) => {
            let wrapper = match kind {
                BodyKind::Json => paths.plugin("axum::Json"),
                BodyKind::Form => paths.plugin("axum::extract::Form"),
            };
            let wrapped = quote!(#wrapper<#inner>);

            (
                Some(inner),
                Some(wrapped.clone()),
                quote!(#wrapper(request)),
                wrapped,
            )
        }

        None => (None, None, quote!(()), quote!(())),
    };

    // Build the `http::Request<B>`: verb + URI, the body's content type (when any), then the
    // typed body. Header/URI are valid by construction, so the builder cannot fail here.
    let request_builder = quote! {{
        let mut __builder = #http::Request::builder()
            .method(#http::Method::#verb)
            .uri(#uri);

        if let ::core::option::Option::Some(__ct) = <#body_ty as #http_body>::CONTENT_TYPE {
            __builder = __builder.header(#http::header::CONTENT_TYPE, __ct);
        }

        __builder
            .body(#body_value)
            .expect("client request is valid by construction")
    }};

    let request_envelope = Some(quote!(#http::Request<#body_ty>));
    let response_envelope = Some(quote!(#http_response<#response>));

    ClientMethod {
        ident: method_ident.clone(),
        // Empty: the method and the full URI live in the `http::Request` envelope the
        // `request_builder` constructs, so the capability's `path` arg carries nothing for HTTP
        // (it exists for RPC's `"Service.method"` routing). The transport reads `request.uri()`.
        path: String::new(),
        capability: Capability::Unary,
        request,
        encode_as,
        req_item: None,
        resp_item: None,
        response,
        error_ty: None,
        extra_args: path_plan.args,
        request_envelope,
        request_builder: Some(request_builder),
        response_envelope,
        response_mapper: None,
        override_bounds: None,
        override_ret: None,
        override_body: None,
    }
}

/// The decoded response body type: the `T` of a `Json<T>` return (the common case), or the bare
/// return type, or `()` for no return.
fn response_type(output: &ReturnType) -> Type {
    match output {
        ReturnType::Type(_, ty) => first_type_arg(ty, "Json").unwrap_or_else(|| (**ty).clone()),

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
    use super::parse_template;

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
}
