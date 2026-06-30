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

/// The WebSocket message attribute: `#[message("destination")]`. Claimed by `#[handlers]` on a ws
/// controller (`#[controller(ws = ..)]`); on its own it emits a `compile_error!` like the verbs.
pub const MESSAGE_ATTR: &str = "message";

/// Whether `attr` is a `#[message(..)]` attribute.
pub fn is_message_attr(attr: &Attribute) -> bool {
    attr.path().is_ident(MESSAGE_ATTR)
}

/// Parses `#[message("destination")]` into its destination literal.
pub fn parse_message_attr(attr: &Attribute) -> syn::Result<LitStr> {
    attr.parse_args()
}

/// A parsed route binding: the `axum::routing` verb to mount under, the (relative) path, and
/// whether the route's response is streamed.
pub struct RouteAttr {
    /// The lowercase verb ident (`get`, `post`, …) naming the `axum::routing` constructor.
    pub verb: Ident,

    /// The route path, relative to the controller's base (e.g. `"/{id}"`, or `""` for the base).
    pub path: LitStr,

    /// The `streamed` flag: the handler's return is a streamed response (a concrete stream type,
    /// or a wrapper like `Sse<impl Stream>` / `Ndjson<impl Stream>`). It marks the route as
    /// server-streaming for the client; the framing comes from the return type, never hard-wired.
    pub streamed: bool,
}

/// The arguments of the raw `#[route(METHOD, "/path"[, streamed])]` attribute.
struct RouteArgs {
    method: Ident,
    path: LitStr,
    streamed: bool,
}

impl Parse for RouteArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let method: Ident = input.parse()?;
        input.parse::<Token![,]>()?;
        let path: LitStr = input.parse()?;
        let streamed = parse_trailing_streamed(input)?;

        Ok(RouteArgs {
            method,
            path,
            streamed,
        })
    }
}

/// The arguments of a verb shorthand (`#[get("/path"[, streamed])]`, `#[get(streamed)]`, `#[get]`).
struct ShorthandArgs {
    path: LitStr,
    streamed: bool,
}

impl Parse for ShorthandArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // A bare `streamed` (no path) mounts at the controller base and streams.
        if input.peek(Ident) {
            expect_streamed(input)?;

            return Ok(ShorthandArgs {
                path: LitStr::new("", input.span()),
                streamed: true,
            });
        }

        let path: LitStr = input.parse()?;
        let streamed = parse_trailing_streamed(input)?;

        Ok(ShorthandArgs { path, streamed })
    }
}

/// Consumes an optional trailing `, streamed`.
fn parse_trailing_streamed(input: ParseStream) -> syn::Result<bool> {
    if input.is_empty() {
        return Ok(false);
    }

    input.parse::<Token![,]>()?;
    expect_streamed(input)?;

    Ok(true)
}

/// Parses the `streamed` keyword, erroring on any other identifier.
fn expect_streamed(input: ParseStream) -> syn::Result<()> {
    let ident: Ident = input.parse()?;

    if ident != "streamed" {
        return Err(syn::Error::new_spanned(
            &ident,
            "unknown route flag; the only flag is `streamed`",
        ));
    }

    Ok(())
}

/// Whether `ident` names a route attribute this crate claims.
pub fn is_route_attr(attr: &Attribute) -> bool {
    attr.path()
        .get_ident()
        .is_some_and(|ident| ROUTE_ATTRS.iter().any(|name| ident == name))
}

/// Parses a route attribute into its verb, path, and `streamed` flag. The shorthand verb
/// attributes take an optional path literal (absent means the controller base) and an optional
/// `streamed` flag; `route` takes `METHOD, "/path"[, streamed]`.
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
            streamed: args.streamed,
        });
    }

    // A verb shorthand: the verb is the attribute name. `#[get]` with no arguments mounts at the
    // controller base.
    let (path, streamed) = match &attr.meta {
        Meta::Path(_) => (LitStr::new("", attr.path().span()), false),

        _ => {
            let args: ShorthandArgs = attr.parse_args()?;

            (args.path, args.streamed)
        }
    };

    Ok(RouteAttr {
        verb: format_ident!("{}", name),
        path,
        streamed,
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
            "route attributes (#[get], #[post], #[route(..)], #[message(..)], …) are only valid on \
             a method inside a #[handlers] impl of a #[controller]"
        );

        #item
    })
}
