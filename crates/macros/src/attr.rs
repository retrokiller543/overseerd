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

/// Arguments of `#[rpc]` / `#[rpc(command|query|stream)]`.
pub struct RpcArgs {
    pub operation: Option<Ident>,
}

impl Parse for RpcArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(RpcArgs { operation: None });
        }

        let operation: Ident = input.parse()?;

        Ok(RpcArgs {
            operation: Some(operation),
        })
    }
}

/// Maps an optional operation keyword to its `OperationKind` variant ident.
///
/// Only unary RPCs are supported today: a bare `#[rpc]` maps to `Unary`. Any
/// argument is rejected until streaming lands (see the streaming plan).
pub fn operation_variant(operation: &Option<Ident>) -> syn::Result<Ident> {
    match operation {
        None => Ok(Ident::new("Unary", Span::call_site())),
        Some(ident) => Err(syn::Error::new(
            ident.span(),
            "streaming RPCs are not implemented yet (see specs/002-streaming-rpcs/plan.md); \
             use a bare `#[rpc]` for a unary method",
        )),
    }
}

/// The logical response *body* type, for descriptor metadata only.
///
/// Handlers may return any [`Responder`](overseer_core::Responder): a bare value,
/// `Result<T, E>`, `ResponseStream<T>`, `()`, etc. This peels the wrappers that
/// carry a body to its inner type — `Result<T, _>` and `ResponseStream<T>` both
/// yield `T` — and reports `()` for an absent return. It never fails: dispatch
/// is uniform regardless of the shape, so this only feeds the `output` field.
pub fn response_body_type(output: &ReturnType) -> Type {
    let ty = match output {
        ReturnType::Default => return syn::parse_quote!(()),
        ReturnType::Type(_, ty) => ty.as_ref(),
    };

    if let Type::Path(path) = ty
        && let Some(segment) = path.path.segments.last()
        && (segment.ident == "Result" || segment.ident == "ResponseStream")
        && let PathArguments::AngleBracketed(generics) = &segment.arguments
        && let Some(GenericArgument::Type(inner)) = generics.args.first()
    {
        return inner.clone();
    }

    ty.clone()
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
