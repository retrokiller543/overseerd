//! Route-attribute parsing for `#[handlers]` controller methods.
//!
//! A controller method is bound to a route by a verb shorthand — `#[get("/{id}")]`,
//! `#[post("/")]`, … — or the raw `#[route(METHOD, "/path")]` for verbs without a shorthand
//! or for clarity. `#[handlers]` strips and parses these; used on their own (outside a
//! `#[handlers]` impl) the shorthand attributes emit a `compile_error!`, like `#[rpc]`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Attribute, Ident, ItemFn, LitStr, Meta, Token};

/// The verb shorthands plus the raw `route` attribute, all of which `#[handlers]` claims on a
/// method. Used to find a method's route attribute among its attrs.
pub const ROUTE_ATTRS: &[&str] = &[
    "get", "post", "put", "delete", "patch", "head", "options", "route",
];

/// The HTTP verbs that have an `axum::routing::<verb>` constructor (and a matching
/// `MethodRouter::<verb>` chaining method).
const VERBS: &[&str] = &["get", "post", "put", "delete", "patch", "head", "options"];

/// A parsed route binding: the `axum::routing` verb to mount under, and the (relative) path.
pub struct RouteAttr {
    /// The lowercase verb ident (`get`, `post`, …) naming the `axum::routing` constructor.
    pub verb: Ident,

    /// The route path, relative to the controller's base (e.g. `"/{id}"`, or `""` for the base).
    pub path: LitStr,
}

/// The arguments of the raw `#[route(METHOD, "/path")]` attribute.
struct RouteArgs {
    method: Ident,
    path: LitStr,
}

impl Parse for RouteArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let method: Ident = input.parse()?;
        input.parse::<Token![,]>()?;
        let path: LitStr = input.parse()?;

        Ok(RouteArgs { method, path })
    }
}

/// Whether `ident` names a route attribute this crate claims.
pub fn is_route_attr(attr: &Attribute) -> bool {
    attr.path()
        .get_ident()
        .is_some_and(|ident| ROUTE_ATTRS.iter().any(|name| ident == name))
}

/// Parses a route attribute into its verb and path. The shorthand verb attributes take an
/// optional path literal (absent means the controller base); `route` takes `METHOD, "/path"`.
pub fn parse_route_attr(attr: &Attribute) -> syn::Result<RouteAttr> {
    let name = attr
        .path()
        .get_ident()
        .map(Ident::to_string)
        .unwrap_or_default();

    if name == "route" {
        let args: RouteArgs = attr.parse_args()?;
        let verb = normalize_verb(&args.method)?;

        return Ok(RouteAttr {
            verb,
            path: args.path,
        });
    }

    // A verb shorthand: the verb is the attribute name. The path is optional — `#[get]` with
    // no arguments mounts at the controller base.
    let path = match &attr.meta {
        Meta::Path(_) => LitStr::new("", attr.path().span()),
        _ => attr.parse_args::<LitStr>()?,
    };

    Ok(RouteAttr {
        verb: format_ident!("{}", name),
        path,
    })
}

/// Lowercases and validates a raw `route` verb against the supported set.
fn normalize_verb(method: &Ident) -> syn::Result<Ident> {
    let lowered = method.to_string().to_lowercase();

    if !VERBS.contains(&lowered.as_str()) {
        return Err(syn::Error::new_spanned(
            method,
            format!(
                "unsupported HTTP method `{method}`; expected one of: {}",
                VERBS.join(", ")
            ),
        ));
    }

    Ok(format_ident!("{}", lowered))
}

/// The standalone expansion of a verb-shorthand attribute: a `compile_error!` plus the
/// original item, since the attribute is only meaningful when claimed by `#[handlers]`.
pub fn expand_standalone(item: ItemFn) -> syn::Result<TokenStream> {
    Ok(quote! {
        ::core::compile_error!(
            "route attributes (#[get], #[post], #[route(..)], …) are only valid on a method \
             inside a #[handlers] impl of a #[controller]"
        );

        #item
    })
}
