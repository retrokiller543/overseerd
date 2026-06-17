//! Attribute-argument parsing and small `syn` helpers shared by the macros.
//!
//! All parsing returns `syn::Result` with spanned errors so diagnostics point
//! at the offending token rather than the whole attribute.

use proc_macro2::Span;
use syn::{
    GenericArgument, Ident, LitStr, PathArguments, ReturnType, Token, Type,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

/// Arguments of `#[service(id = "..", name = "..", version = "..")]`. All keys
/// are optional; `id`/`name` default to the impl's type name.
pub struct ServiceArgs {
    pub id: Option<LitStr>,
    pub name: Option<LitStr>,
    pub version: Option<LitStr>,
}

impl Parse for ServiceArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let pairs = Punctuated::<KeyValue, Token![,]>::parse_terminated(input)?;

        let mut args = ServiceArgs {
            id: None,
            name: None,
            version: None,
        };

        for pair in pairs {
            let key = pair.key.to_string();

            match key.as_str() {
                "id" => args.id = Some(pair.value),
                "name" => args.name = Some(pair.value),
                "version" => args.version = Some(pair.value),
                _ => {
                    return Err(syn::Error::new(
                        pair.key.span(),
                        format!(
                            "unknown service argument `{key}`, expected `id`, `name`, or `version`"
                        ),
                    ));
                }
            }
        }

        Ok(args)
    }
}

/// A single `key = "value"` pair inside `#[service(...)]`.
struct KeyValue {
    key: Ident,
    value: LitStr,
}

impl Parse for KeyValue {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let key: Ident = input.parse()?;
        let _: Token![=] = input.parse()?;
        let value: LitStr = input.parse()?;

        Ok(KeyValue { key, value })
    }
}

/// Maps the `(streamed_input, streamed_output)` pair inferred from a handler
/// signature to its `OperationKind` variant ident. The kind is structural, so
/// `#[rpc]` derives it rather than taking an annotation.
pub fn operation_ident(streamed_input: bool, streamed_output: bool) -> Ident {
    let name = match (streamed_input, streamed_output) {
        (false, false) => "Unary",
        (false, true) => "ServerStream",
        (true, false) => "ClientStream",
        (true, true) => "BidiStream",
    };

    Ident::new(name, Span::call_site())
}

/// Whether a return type's body is a `ResponseStream<T>` (after peeling an
/// optional outer `Result`), i.e. the handler streams its output.
pub fn returns_response_stream(output: &ReturnType) -> bool {
    let ReturnType::Type(_, ty) = output else {
        return false;
    };

    type_name(peel_named(ty, "Result")).is_some_and(|id| id == "ResponseStream")
}

/// Whether a parameter type is the inbound-stream extractor `Streaming<T>`.
pub fn is_streaming_param(ty: &Type) -> bool {
    type_name(ty).is_some_and(|id| id == "Streaming")
}

/// Whether a parameter type is the single-body extractor `Payload<T>`.
pub fn is_payload_param(ty: &Type) -> bool {
    type_name(ty).is_some_and(|id| id == "Payload")
}

/// The logical response *body* type, for descriptor metadata only.
///
/// Handlers may return any [`Responder`](overseer_core::Responder): a bare value,
/// `Result<T, E>`, `ResponseStream<T>`, `Result<ResponseStream<T>, E>`, `()`,
/// etc. This peels the `Result` and `ResponseStream` wrappers to the body type,
/// and reports `()` for an absent return. It never fails: dispatch is uniform
/// regardless of the shape, so this only feeds the `output` field.
pub fn response_body_type(output: &ReturnType) -> Type {
    let ty = match output {
        ReturnType::Default => return syn::parse_quote!(()),
        ReturnType::Type(_, ty) => ty.as_ref(),
    };

    peel_named(peel_named(ty, "Result"), "ResponseStream").clone()
}

/// The last path-segment ident of a simple path type (e.g. `Foo` of `a::b::Foo<T>`).
fn type_name(ty: &Type) -> Option<&Ident> {
    match ty {
        Type::Path(path) => path.path.segments.last().map(|segment| &segment.ident),
        _ => None,
    }
}

/// If `ty` is `Name<T, ..>`, returns its first type argument `T`; otherwise `ty`.
fn peel_named<'a>(ty: &'a Type, name: &str) -> &'a Type {
    if let Type::Path(path) = ty
        && let Some(segment) = path.path.segments.last()
        && segment.ident == name
        && let PathArguments::AngleBracketed(generics) = &segment.arguments
        && let Some(GenericArgument::Type(inner)) = generics.args.first()
    {
        return inner;
    }

    ty
}

/// Extracts `T` from `Arc<T>` (the form `#[init]` dependency parameters take).
pub fn arc_inner_type(ty: &Type) -> syn::Result<Type> {
    let err = || {
        syn::Error::new_spanned(
            ty,
            "#[init] parameters must be `Arc<Component>` dependencies",
        )
    };

    let Type::Path(path) = ty else {
        return Err(err());
    };

    let segment = path.path.segments.last().ok_or_else(err)?;

    if segment.ident != "Arc" {
        return Err(err());
    }

    let PathArguments::AngleBracketed(generics) = &segment.arguments else {
        return Err(err());
    };

    match generics.args.first() {
        Some(GenericArgument::Type(inner)) => Ok(inner.clone()),
        _ => Err(err()),
    }
}

/// Whether a return type is a `Result<...>` (vs. an infallible bare value).
pub fn returns_result(output: &ReturnType) -> bool {
    let ReturnType::Type(_, ty) = output else {
        return false;
    };

    let Type::Path(path) = ty.as_ref() else {
        return false;
    };

    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "Result")
}
