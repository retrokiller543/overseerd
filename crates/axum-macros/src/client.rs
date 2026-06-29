//! Building the protocol-agnostic [`ClientMethod`] hint for an HTTP route.
//!
//! The framework owns client generation (in `macros-core`); a protocol only describes each
//! method as a hint. For HTTP that means: classify the handler's extractor args into client
//! inputs (a `Path` param, a `Json`/`Form` body) vs server-only ones (`Inject`/`State`/
//! `Extension`, dropped), then fill the hint's `request_builder` (construct an `http::Request`
//! with the verb, the `BASE`+route URI, and the typed body) and the request/response envelopes
//! (`http::Request<B>` / `HttpResponse<R>`). The framework assembles the rest.
//!
//! A route whose args are not all classifiable yields `None` — the server route still
//! registers, it simply gets no generated client method (rather than a silently wrong one).

use overseerd_macros_core::attr::{first_type_arg, type_name};
use overseerd_macros_core::client::{Capability, ClientMethod};
use overseerd_macros_core::paths::Paths;
use quote::{format_ident, quote};
use syn::{Ident, ReturnType, Type};

use crate::route::RouteAttr;

/// Server-only extractors: resolved on the server, never sent by the client, so they are
/// dropped from the client signature (the method is still emitted).
const SERVER_ONLY: &[&str] = &["Inject", "State", "Extension", "ConnectInfo"];

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
    let mut path_param: Option<Type> = None;
    let mut body: Option<Type> = None;

    for ty in arg_types {
        if let Some(inner) = first_type_arg(ty, "Path") {
            if path_param.is_some() {
                return None;
            }

            path_param = Some(inner);

            continue;
        }

        if first_type_arg(ty, "Json").is_some() || first_type_arg(ty, "Form").is_some() {
            if body.is_some() {
                return None;
            }

            // Keep the wrapper type itself (`Json<T>` / `Form<T>`): it is the `HttpBody` that
            // carries the content type and encoding.
            body = Some((*ty).clone());

            continue;
        }

        // Server-only extractors are dropped; anything else is unrecognized, so skip the client
        // method rather than emit one that silently omits an input.
        match type_name(ty).map(Ident::to_string).as_deref() {
            Some(name) if SERVER_ONLY.contains(&name) => continue,

            _ => return None,
        }
    }

    Some(assemble(
        controller,
        method_ident,
        route,
        path_param,
        body,
        output,
        paths,
    ))
}

/// Assembles the hint once the args are classified.
fn assemble(
    controller: &Ident,
    method_ident: &Ident,
    route: &RouteAttr,
    path_param: Option<Type>,
    body: Option<Type>,
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
    // a positional `{}`, filled from the single `Path` parameter (scalar, or a tuple element per
    // hole). One `http::Method::<VERB>` selects the verb.
    let (fmt, holes) = template_format(&route.path.value());
    let verb = format_ident!("{}", route.verb.to_string().to_uppercase());
    let base = quote!(<#controller as #controller_trait>::BASE);

    let subst = match holes {
        0 => quote!(),
        1 => quote!(, path),
        n => {
            let idx = (0..n).map(syn::Index::from);

            quote!(#(, path.#idx)*)
        }
    };
    let uri = quote!(::std::format!(#fmt, #base #subst));

    // The body wrapper (`Json<T>`/`Form<T>`) drives the `Encodes<B>` bound and the envelope; a
    // no-body route uses `()`.
    let body_ty = body
        .clone()
        .map(|ty| quote!(#ty))
        .unwrap_or_else(|| quote!(()));
    let body_value = if body.is_some() {
        quote!(request)
    } else {
        quote!(())
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

    // The leading path parameter, named `path`, ahead of the body argument.
    let extra_args = match path_param {
        Some(ty) => vec![(format_ident!("path"), quote!(#ty))],
        None => Vec::new(),
    };

    // Built before the struct literal moves `response` into its field.
    let request_envelope = Some(quote!(#http::Request<#body_ty>));
    let response_envelope = Some(quote!(#http_response<#response>));

    ClientMethod {
        ident: method_ident.clone(),
        // Unused by HTTP transports (the URI lives in the built request), but kept for logging.
        path: format!("{} {}", route.verb, route.path.value()),
        capability: Capability::Unary,
        request: body,
        req_item: None,
        resp_item: None,
        response,
        error_ty: None,
        extra_args,
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

/// Turns a route template into a `format!` string and the hole count: each `{name}` becomes a
/// positional `{}`. The leading `{}` for `BASE` is prepended by the caller's args, so the
/// returned string starts with `{}` then the templated remainder.
fn template_format(template: &str) -> (String, usize) {
    let mut out = String::from("{}");
    let mut holes = 0;
    let mut chars = template.chars();

    while let Some(c) = chars.next() {
        if c == '{' {
            // Consume up to the closing brace and emit one positional placeholder.
            for inner in chars.by_ref() {
                if inner == '}' {
                    break;
                }
            }

            out.push_str("{}");
            holes += 1;
        } else {
            out.push(c);
        }
    }

    (out, holes)
}
