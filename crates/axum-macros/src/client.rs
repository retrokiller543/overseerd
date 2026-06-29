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
use syn::{Ident, ReturnType, Type};

use crate::route::RouteAttr;

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

        // Server-only extractors are dropped; anything else is unrecognized, so skip the client
        // method rather than emit one that silently omits an input.
        match type_name(ty).map(Ident::to_string).as_deref() {
            Some(name) if SERVER_ONLY.contains(&name) => continue,

            _ => return None,
        }
    }

    let (fmt, holes) = parse_template(&route.path.value());
    let path_plan = plan_path(&holes, path_ty)?;

    Some(assemble(
        controller,
        method_ident,
        route,
        fmt,
        path_plan,
        body,
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
