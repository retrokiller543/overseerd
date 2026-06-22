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

/// Arguments of `#[handlers(client_trait = Name)]`. The only key is the optional
/// `client_trait`: when present the generated client is emitted as a trait `Name`
/// plus its impl (mockable, `dyn`-compatible); when absent, as a plain inherent impl.
pub struct HandlersArgs {
    pub client_trait: Option<Ident>,
}

impl Parse for HandlersArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut client_trait = None;

        let args = Punctuated::<HandlersArg, Token![,]>::parse_terminated(input)?;

        for arg in args {
            match arg {
                HandlersArg::ClientTrait(ident) => client_trait = Some(ident),
            }
        }

        Ok(HandlersArgs { client_trait })
    }
}

/// A single recognized `#[handlers(...)]` argument.
enum HandlersArg {
    ClientTrait(Ident),
}

impl Parse for HandlersArg {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let key: Ident = input.parse()?;
        let _: Token![=] = input.parse()?;

        match key.to_string().as_str() {
            "client_trait" => {
                let value: Ident = input.parse()?;

                Ok(HandlersArg::ClientTrait(value))
            }
            other => Err(syn::Error::new(
                key.span(),
                format!("unknown handlers argument `{other}`, expected `client_trait`"),
            )),
        }
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

/// The first type argument of `Name<T, ..>` when the last path segment is `name`.
/// Used by the client codegen to recover request/response payload types from the
/// extractor and `Responder` wrappers in a handler signature.
fn first_type_arg(ty: &Type, name: &str) -> Option<Type> {
    if let Type::Path(path) = ty
        && let Some(segment) = path.path.segments.last()
        && segment.ident == name
        && let PathArguments::AngleBracketed(generics) = &segment.arguments
        && let Some(GenericArgument::Type(inner)) = generics.args.first()
    {
        return Some(inner.clone());
    }

    None
}

/// The request body type `T` of a `Payload<T>` parameter.
pub fn payload_inner(ty: &Type) -> Option<Type> {
    first_type_arg(ty, "Payload")
}

/// The request item type `T` of a `Streaming<T>` parameter.
pub fn streaming_inner(ty: &Type) -> Option<Type> {
    first_type_arg(ty, "Streaming")
}

/// The response item type `T` of a `ResponseStream<T>` return.
pub fn response_stream_inner(ty: &Type) -> Option<Type> {
    first_type_arg(ty, "ResponseStream")
}

/// If `output` is `Result<Ok, Err?>`, returns `(Ok, Err)`. `Err` is `None` for a
/// one-argument alias such as `overseer::Result<T>` (whose error is the framework
/// `Error`, opaque to the client and surfaced as a raw body).
pub fn result_type_args(output: &ReturnType) -> Option<(Type, Option<Type>)> {
    let ReturnType::Type(_, ty) = output else {
        return None;
    };

    let Type::Path(path) = ty.as_ref() else {
        return None;
    };

    let segment = path.path.segments.last()?;

    if segment.ident != "Result" {
        return None;
    }

    let PathArguments::AngleBracketed(generics) = &segment.arguments else {
        return None;
    };

    let mut args = generics.args.iter().filter_map(|arg| match arg {
        GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    });

    let ok = args.next()?;
    let err = args.next();

    Some((ok, err))
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
