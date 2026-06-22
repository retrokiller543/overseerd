//! Attribute-argument parsing and small `syn` helpers shared by the macros.
//!
//! All parsing returns `syn::Result` with spanned errors so diagnostics point
//! at the offending token rather than the whole attribute.

use proc_macro2::Span;
use syn::{
    GenericArgument, Ident, LitStr, PathArguments, ReturnType, Token, Type,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    token,
};

/// Arguments of `#[service(...)]` / `#[component(...)]`. All optional:
/// - `id` / `name` — default to the type name (`name` is also the RPC prefix);
/// - `version` — service version;
/// - `provide = dyn Trait` or `provide = [dyn A, dyn B]` — traits this component
///   provides, injectable as `Arc<dyn Trait>` / `Vec<_>` / `HashMap<String, _>`.
///   The trait must be `Send + Sync` (state it as a supertrait: `trait Trait:
///   Send + Sync`), so no use site needs to write `+ Send + Sync`;
/// - `qualifier = ".."` — key for `HashMap<String, Arc<dyn Trait>>` injection;
/// - `primary` — mark this the primary provider for every trait it provides;
/// - `by_value` — store/inject this component as `Self` rather than `Arc<Self>`
///   (for cheap-to-clone, typically internally-`Arc`, types).
#[derive(Default)]
pub struct ServiceArgs {
    pub id: Option<LitStr>,
    pub name: Option<LitStr>,
    pub version: Option<LitStr>,
    pub provide: Vec<Type>,
    pub qualifier: Option<LitStr>,
    pub primary: bool,
    pub by_value: bool,
}

impl Parse for ServiceArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = ServiceArgs::default();

        while !input.is_empty() {
            let key: Ident = input.parse()?;

            match key.to_string().as_str() {
                "id" => args.id = Some(parse_value(input)?),
                "name" => args.name = Some(parse_value(input)?),
                "version" => args.version = Some(parse_value(input)?),
                "qualifier" => args.qualifier = Some(parse_value(input)?),
                "primary" => args.primary = true,
                "by_value" => args.by_value = true,
                "provide" => {
                    input.parse::<Token![=]>()?;

                    if input.peek(token::Bracket) {
                        let content;
                        syn::bracketed!(content in input);
                        let list =
                            Punctuated::<ProvidedTrait, Token![,]>::parse_terminated(&content)?;
                        args.provide.extend(list.into_iter().map(|t| t.0));
                    } else {
                        args.provide.push(input.parse::<ProvidedTrait>()?.0);
                    }
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown argument `{other}`, expected `id`, `name`, `version`, \
                             `provide`, `qualifier`, `primary`, or `by_value`"
                        ),
                    ));
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(args)
    }
}

/// Parses the `= "value"` half of a string-valued argument.
fn parse_value(input: ParseStream) -> syn::Result<LitStr> {
    input.parse::<Token![=]>()?;

    input.parse()
}

/// A provided-trait entry, parsed as a trait-object [`Type`] (`dyn Trait`) so the
/// IDE treats it as a type — completions, go-to-definition, and highlighting —
/// rather than an opaque path. The trait must be `Send + Sync` via supertraits.
struct ProvidedTrait(Type);

impl Parse for ProvidedTrait {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ty: Type = input.parse()?;

        if !matches!(ty, Type::TraitObject(_)) {
            return Err(syn::Error::new_spanned(
                &ty,
                "`provide` expects a trait object, e.g. `dyn Repo`",
            ));
        }

        Ok(ProvidedTrait(ty))
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

/// The handle type `H` of an `Option<H>` field (an optional dependency).
pub fn option_inner(ty: &Type) -> Option<Type> {
    first_type_arg(ty, "Option")
}

/// The item handle `H` of a `Vec<H>` field (a collection of all trait providers).
pub fn vec_inner(ty: &Type) -> Option<Type> {
    first_type_arg(ty, "Vec")
}

/// The value handle `H` of a `HashMap<String, H>` field (qualifier-keyed trait
/// providers). Only matches when the key is `String`.
pub fn hashmap_value(ty: &Type) -> Option<Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;

    if segment.ident != "HashMap" {
        return None;
    }

    let PathArguments::AngleBracketed(generics) = &segment.arguments else {
        return None;
    };

    let mut types = generics.args.iter().filter_map(|arg| match arg {
        GenericArgument::Type(t) => Some(t),
        _ => None,
    });
    let key = types.next()?;
    let value = types.next()?;

    if type_name(key).is_some_and(|id| id == "String") {
        return Some(value.clone());
    }

    None
}

/// The handle type `H` of a `Dynamic<H>` field (a runtime-provided dependency).
pub fn dynamic_inner(ty: &Type) -> Option<Type> {
    first_type_arg(ty, "Dynamic")
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
