use overseerd_macros_core::attr::{first_type_arg, type_name};
use overseerd_macros_core::paths::Paths;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{GenericArgument, Ident, PathArguments, ReturnType, Type};

use super::response::{is_opaque_response, response_type};

/// The classified client inputs of a route: an optional `Path` type, query, and request body.
pub(crate) struct Inputs {
    pub(crate) path_ty: Option<Type>,
    pub(super) query: Option<QueryInput>,
    pub(crate) body: Option<Body>,
}

/// A typed `Query<T>` or an untyped raw query string.
#[allow(clippy::large_enum_variant)]
pub(super) enum QueryInput {
    Typed(Type),
    Raw,
}

/// A route request body and, for serde bodies, its payload type.
pub(crate) struct Body {
    pub(crate) kind: BodyKind,
    pub(crate) inner: Option<Type>,
}

/// Which client `HttpBody` wrapper a route body uses.
pub(crate) enum BodyKind {
    Json,
    Form,
    Bytes,
    RawForm,
    Multipart,
}

/// Classifies wire inputs while dropping server-only request context.
///
/// A route opts out when two arguments occupy the same wire slot or an extractor in
/// [`UNSUPPORTED_WIRE`] carries data the generated client cannot encode faithfully.
pub(crate) fn classify(arg_types: &[&Type]) -> Option<Inputs> {
    let mut path_ty = None;
    let mut query = None;
    let mut body = None;

    let set_body = |slot: &mut Option<Body>, kind, inner| {
        if slot.is_some() {
            return None;
        }

        *slot = Some(Body { kind, inner });
        Some(())
    };

    for ty in arg_types {
        let ty = peel_request_combinator(ty);

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

        if let Some(inner) =
            first_type_arg(ty, "Json").or_else(|| first_type_arg(ty, "JsonDeserializer"))
        {
            set_body(&mut body, BodyKind::Json, Some(inner))?;
            continue;
        }

        if let Some(inner) = first_type_arg(ty, "Form") {
            set_body(&mut body, BodyKind::Form, Some(inner))?;
            continue;
        }

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
            Some(name) if UNSUPPORTED_WIRE.contains(&name) => return None,
            Some(name) if SERVER_CONTEXT.contains(&name) => {}
            // Custom request-parts extractors retain the established server-context behavior.
            _ => {}
        }
    }

    Some(Inputs {
        path_ty,
        query,
        body,
    })
}

fn peel_request_combinator(ty: &Type) -> &Type {
    let mut current = ty;

    loop {
        let name = type_name(current).map(Ident::to_string);
        let next = match name.as_deref() {
            Some("Cached" | "WithRejection") => first_type_arg_ref(current),
            _ => None,
        };

        match next {
            Some(inner) => current = inner,
            None => return current,
        }
    }
}

fn first_type_arg_ref(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };

    arguments.args.iter().find_map(|argument| match argument {
        GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

/// Collects route wire types that must implement `Dto`.
pub(crate) fn collect_wire_types(arg_types: &[&Type], output: &ReturnType, sink: &mut Vec<Type>) {
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

    if let Some(Body {
        inner: Some(body), ..
    }) = inputs.body
    {
        sink.push(body);
    }

    let response = response_type(output);

    if !is_opaque_response(&response) {
        sink.push(response);
    }
}

/// Emits one deduplicated assertion block for collected `Dto` wire types.
pub(super) fn dto_assertions(mut wire_types: Vec<Type>, paths: &Paths) -> TokenStream {
    if wire_types.is_empty() {
        return quote!();
    }

    let mut seen = std::collections::HashSet::<String>::new();
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

/// Extractors that carry unsupported wire data and must opt out of client generation.
const UNSUPPORTED_WIRE: &[&str] = &[
    "Request",
    "RawRequest",
    "Either",
    "Either3",
    "Either4",
    "Either5",
    "Either6",
    "Either7",
    "Either8",
    "JsonLines",
    "OptionalPath",
    "OptionalQuery",
    "Protobuf",
];

/// Known framework and axum-extra server-only request context.
const SERVER_CONTEXT: &[&str] = &[
    "ConnectInfo",
    "Cached",
    "CookieJar",
    "Extension",
    "Host",
    "HeaderMap",
    "Inject",
    "Method",
    "OriginalUri",
    "PrivateCookieJar",
    "Scheme",
    "SignedCookieJar",
    "State",
    "TypedHeader",
    "WithRejection",
    "Uri",
    "Version",
];
