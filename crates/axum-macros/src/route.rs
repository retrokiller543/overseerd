//! Route-attribute parsing for `#[handlers]` controller methods.
//!
//! A controller method is bound to a route by a verb shorthand — `#[get("/{id}")]`,
//! `#[post("/")]`, … — or the raw `#[route(METHOD, "/path")]` for verbs without a shorthand
//! or for clarity. `#[handlers]` strips and parses these; used on their own (outside a
//! `#[handlers]` impl) the shorthand attributes emit a `compile_error!`, like `#[rpc]`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Attribute, Ident, ItemFn, LitStr, Meta, Path, Token, bracketed};

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

/// How a `#[message]` handler's client method is shaped. Inferred from the handler's return type
/// (unit → [`Send`](MessageMode::Send), non-unit → [`Request`](MessageMode::Request)) unless the
/// attribute names it explicitly — mirroring how `#[rpc(stream)]` overrides RPC capability inference.
///
/// Because inference keys purely on unit-vs-non-unit, a handler that returns a value for a reason
/// *other* than replying to the caller — e.g. returning a `Publish`/`Vec<Publish>` to imperatively
/// broadcast — is inferred as a request and will not compile (a broadcast value is not the reply
/// type the client decodes). Annotate such a handler `#[message(send)]` to force the fire-and-forget
/// path. (The idiomatic imperative-broadcast route is an injected `Publisher`, which returns `()`.)
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MessageMode {
    /// Infer from the return type: `()` is a fire-and-forget SEND, anything else a request/response.
    Infer,

    /// Force a fire-and-forget SEND. The return is routed through the protocol's SEND path (for
    /// STOMP: `()`, a `Publish`/`Vec<Publish>` to broadcast, a `StompOutcome`, or a `Result` of
    /// those) — never sent back to the caller. Forcing `send` on a handler returning an arbitrary
    /// reply DTO is therefore a compile error; use the inferred/`request` mode for that.
    Send,

    /// Force a request/response: the handler's return is routed back to the requester.
    Request,
}

/// A parsed `#[message("destination"[, send|request])]`: the destination literal and the mode.
pub struct MessageArgs {
    /// The message destination (e.g. `"/app/chat"`).
    pub destination: LitStr,

    /// The explicit or inferred SEND-vs-request mode.
    pub mode: MessageMode,
}

impl Parse for MessageArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let destination: LitStr = input.parse()?;

        let mode = if input.is_empty() {
            MessageMode::Infer
        } else {
            input.parse::<Token![,]>()?;
            let keyword: Ident = input.parse()?;

            match keyword.to_string().as_str() {
                "send" => MessageMode::Send,
                "request" => MessageMode::Request,

                _ => {
                    return Err(syn::Error::new_spanned(
                        &keyword,
                        "unknown #[message] flag; expected `send` or `request`",
                    ));
                }
            }
        };

        Ok(MessageArgs { destination, mode })
    }
}

/// Parses `#[message("destination"[, send|request])]` into its destination and mode.
pub fn parse_message_attr(attr: &Attribute) -> syn::Result<MessageArgs> {
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

    /// `middleware = [Type, ..]` — DI-backed [`AxumMiddleware`](../overseerd_axum/trait.AxumMiddleware.html)
    /// singletons scoped to just this route, first-listed outermost. Empty if unset.
    pub middleware: Vec<Path>,
}

/// The arguments of the raw `#[route(METHOD, "/path"[, modifiers])]` attribute.
struct RouteArgs {
    method: Ident,
    path: LitStr,
    streamed: bool,
    middleware: Vec<Path>,
}

impl Parse for RouteArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let method: Ident = input.parse()?;
        input.parse::<Token![,]>()?;
        let path: LitStr = input.parse()?;
        let (streamed, middleware) = parse_trailing_modifiers(input)?;

        Ok(RouteArgs {
            method,
            path,
            streamed,
            middleware,
        })
    }
}

/// The arguments of a verb shorthand (`#[get("/path"[, modifiers])]`, `#[get(streamed)]`, `#[get]`).
struct ShorthandArgs {
    path: LitStr,
    streamed: bool,
    middleware: Vec<Path>,
}

impl Parse for ShorthandArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // Bare modifiers (no path) mount at the controller base.
        if input.peek(Ident) {
            let (streamed, middleware) = parse_modifiers(input)?;

            return Ok(ShorthandArgs {
                path: LitStr::new("", input.span()),
                streamed,
                middleware,
            });
        }

        let path: LitStr = input.parse()?;
        let (streamed, middleware) = parse_trailing_modifiers(input)?;

        Ok(ShorthandArgs {
            path,
            streamed,
            middleware,
        })
    }
}

/// Consumes an optional trailing `, <modifiers>` after a required leading argument.
fn parse_trailing_modifiers(input: ParseStream) -> syn::Result<(bool, Vec<Path>)> {
    if input.is_empty() {
        return Ok((false, Vec::new()));
    }

    input.parse::<Token![,]>()?;
    parse_modifiers(input)
}

/// Parses a comma-separated list of route modifiers: the `streamed` flag and/or
/// `middleware = [Type, ..]`, in either order, each at most once.
fn parse_modifiers(input: ParseStream) -> syn::Result<(bool, Vec<Path>)> {
    let mut streamed = false;
    let mut middleware = Vec::new();

    loop {
        let ident: Ident = input.parse()?;

        match ident.to_string().as_str() {
            "streamed" => streamed = true,

            "middleware" => {
                input.parse::<Token![=]>()?;

                let content;
                bracketed!(content in input);
                middleware = Punctuated::<Path, Token![,]>::parse_terminated(&content)?
                    .into_iter()
                    .collect();
            }

            _ => {
                return Err(syn::Error::new_spanned(
                    &ident,
                    "unknown route flag; expected `streamed` or `middleware = [..]`",
                ));
            }
        }

        if input.is_empty() {
            break;
        }

        input.parse::<Token![,]>()?;
    }

    Ok((streamed, middleware))
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
            middleware: args.middleware,
        });
    }

    // A verb shorthand: the verb is the attribute name. `#[get]` with no arguments mounts at the
    // controller base.
    let (path, streamed, middleware) = match &attr.meta {
        Meta::Path(_) => (LitStr::new("", attr.path().span()), false, Vec::new()),

        _ => {
            let args: ShorthandArgs = attr.parse_args()?;

            (args.path, args.streamed, args.middleware)
        }
    };

    Ok(RouteAttr {
        verb: format_ident!("{}", name),
        path,
        streamed,
        middleware,
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
