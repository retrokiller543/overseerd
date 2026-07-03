//! The Overseerd **axum/HTTP controller** macros: `#[controller]`, `#[handlers]`, and the
//! route attributes (`#[get]`, `#[post]`, `#[put]`, `#[delete]`, `#[patch]`, `#[head]`,
//! `#[options]`, and the raw `#[route(METHOD, "/path")]`). They emit `::overseerd::web::*`
//! types, so they live in their own crate rather than the core `overseerd-macros`, built on
//! the shared [`overseerd_macros_core`] codegen.
//!
//! Re-exported through the `overseerd` facade's `web` module; depend on the facade, not this
//! crate directly.
//!
//! - `#[controller]` is a **router component**: a `#[component]` (field-injected singleton)
//!   plus a controller header, its `{Controller}Routes` slice, and its `ControllerDescriptor`.
//! - `#[handlers]` is `MethodArgs<AxumHandlers>` — the base impl macro (`#[methods]`: `#[init]`
//!   + `#[hook]`) plus the route extension, which registers each route-attributed method.
//! - The route attributes mark a method inside a `#[handlers]` impl (stripped by `#[handlers]`).

extern crate proc_macro;

mod client;
mod dto;
mod handlers;
mod route;
mod router;
mod topics;

use overseerd_macros_core::methods::MethodArgs;
use overseerd_macros_core::paths::Paths;
use overseerd_macros_core::run;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use router::ControllerComponent;
use syn::{DeriveInput, ItemEnum, ItemFn, ItemImpl, ItemStruct};

/// The default crate roots for the axum macros. Core is always the `overseerd` facade; the
/// plugin (own-types) root is `::overseerd::axum` when consumed through the facade (the
/// `facade` feature, set by the `overseerd` crate) and the standalone `::overseerd_axum`
/// otherwise — so a direct dependant on `overseerd-axum` gets working codegen.
fn axum_paths() -> Paths {
    if cfg!(feature = "facade") {
        Paths::new(
            syn::parse_quote!(::overseerd),
            syn::parse_quote!(::overseerd::axum),
        )
    } else {
        Paths::new(
            syn::parse_quote!(::overseerd),
            syn::parse_quote!(::overseerd_axum),
        )
    }
}

/// Declares a **controller** — a router component exposing HTTP routes.
///
/// `#[controller]` is `ComponentArgs<AxumRouter>`: a `#[component]` (field-injected singleton)
/// plus the controller surface — a [`Controller`](../overseerd_axum/trait.Controller.html) impl,
/// the controller's `{Controller}Routes` slice, and its `ControllerDescriptor`. Routes are
/// added by `#[handlers]` impls. Accepts the component keys plus `path` (the base path) and
/// `routes_slice`.
#[proc_macro_attribute]
pub fn controller(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = TokenStream2::from(attr);
    let out = match syn::parse2::<ControllerComponent>(attr) {
        Ok(args) => {
            let paths = args.paths(axum_paths());

            run::<ItemStruct, _>(item.into(), |item| {
                overseerd_macros_core::expand_component(args, item, &paths)
            })
        }

        Err(e) => e.into_compile_error(),
    };

    out.into()
}

/// Contributes the route methods (and an optional `#[init]` / `#[hook]`s) of an inherent
/// `impl` block to the controller of `Self`.
///
/// `#[handlers]` is `#[methods]` plus route registration: it claims each route-attributed
/// method into the controller's `{Controller}Routes` slice, while the shared base also handles
/// `#[init]` constructors and `#[hook]` methods. Several `#[handlers]` blocks for one controller
/// merge with no coordination (as long as they do not register the same path). Accepts
/// `routes_slice = ..`.
#[proc_macro_attribute]
pub fn handlers(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = TokenStream2::from(attr);
    let out = match syn::parse2::<MethodArgs<handlers::AxumHandlers>>(attr) {
        Ok(args) => {
            let paths = args.paths(axum_paths());

            run::<ItemImpl, _>(item.into(), |item| {
                overseerd_macros_core::methods::expand(args, item, &paths)
            })
        }

        Err(e) => e.into_compile_error(),
    };

    out.into()
}

/// Marks a type as HTTP **wire data** ([`Dto`](../overseerd_axum/trait.Dto.html)): a request/response
/// body or a path/query parameter. Derives `serde::Serialize`/`Deserialize` (skip with
/// `#[dto(no_serde)]`), derives `tsify::Tsify` on wasm (so the generated client is TypeScript-typed),
/// and implements `Dto`. `#[handlers]` requires every wire position to be a `Dto`, so a forgotten
/// `#[dto]` is a clear error rather than a cascade of serde/`IntoResponse` failures.
#[proc_macro_attribute]
pub fn dto(attr: TokenStream, item: TokenStream) -> TokenStream {
    let paths = axum_paths();
    let out = match syn::parse2::<dto::DtoArgs>(attr.into()) {
        Ok(args) => run::<DeriveInput, _>(item.into(), |item| dto::expand(args, item, &paths)),

        Err(e) => e.into_compile_error(),
    };

    out.into()
}

/// Marks a `GET` route inside a `#[handlers]` impl. A marker consumed and stripped by
/// `#[handlers]`; used on its own it emits a `compile_error!`.
#[proc_macro_attribute]
pub fn get(_attr: TokenStream, item: TokenStream) -> TokenStream {
    run::<ItemFn, _>(item.into(), route::expand_standalone).into()
}

/// Marks a `POST` route inside a `#[handlers]` impl (see [`get`]).
#[proc_macro_attribute]
pub fn post(_attr: TokenStream, item: TokenStream) -> TokenStream {
    run::<ItemFn, _>(item.into(), route::expand_standalone).into()
}

/// Marks a `PUT` route inside a `#[handlers]` impl (see [`get`]).
#[proc_macro_attribute]
pub fn put(_attr: TokenStream, item: TokenStream) -> TokenStream {
    run::<ItemFn, _>(item.into(), route::expand_standalone).into()
}

/// Marks a `DELETE` route inside a `#[handlers]` impl (see [`get`]).
#[proc_macro_attribute]
pub fn delete(_attr: TokenStream, item: TokenStream) -> TokenStream {
    run::<ItemFn, _>(item.into(), route::expand_standalone).into()
}

/// Marks a `PATCH` route inside a `#[handlers]` impl (see [`get`]).
#[proc_macro_attribute]
pub fn patch(_attr: TokenStream, item: TokenStream) -> TokenStream {
    run::<ItemFn, _>(item.into(), route::expand_standalone).into()
}

/// Marks a `HEAD` route inside a `#[handlers]` impl (see [`get`]).
#[proc_macro_attribute]
pub fn head(_attr: TokenStream, item: TokenStream) -> TokenStream {
    run::<ItemFn, _>(item.into(), route::expand_standalone).into()
}

/// Marks an `OPTIONS` route inside a `#[handlers]` impl (see [`get`]).
#[proc_macro_attribute]
pub fn options(_attr: TokenStream, item: TokenStream) -> TokenStream {
    run::<ItemFn, _>(item.into(), route::expand_standalone).into()
}

/// Marks a route by explicit method inside a `#[handlers]` impl: `#[route(METHOD, "/path")]`
/// (see [`get`]). Use it for verbs without a shorthand or for clarity.
#[proc_macro_attribute]
pub fn route(_attr: TokenStream, item: TokenStream) -> TokenStream {
    run::<ItemFn, _>(item.into(), route::expand_standalone).into()
}

/// Marks a WebSocket message handler inside a `#[handlers]` impl of a `#[controller(ws = ..)]`:
/// `#[message("destination")]`. A marker consumed and stripped by `#[handlers]`; used on its own it
/// emits a `compile_error!`.
#[proc_macro_attribute]
pub fn message(_attr: TokenStream, item: TokenStream) -> TokenStream {
    run::<ItemFn, _>(item.into(), route::expand_standalone).into()
}

/// Declares a **STOMP topic set**: an enum whose variants each carry a `#[topic("/topic/..")]`
/// destination and a single payload type. Emits an `impl Topic` (typed server publish) and a
/// `{Enum}Client<C>` with one `subscribe_<variant>()` per topic (typed client subscribe), so the
/// same enum is the single source of truth for both sides. See the crate docs for an example.
///
/// The per-variant `#[topic("/topic/..")]` is an inert helper attribute: `#[topics]` reads and
/// strips it, so it is never resolved on its own.
#[proc_macro_attribute]
pub fn topics(attr: TokenStream, item: TokenStream) -> TokenStream {
    let paths = axum_paths();
    let out = match syn::parse2::<topics::TopicsArgs>(attr.into()) {
        Ok(args) => run::<ItemEnum, _>(item.into(), |item| topics::expand(args, item, &paths)),

        Err(e) => e.into_compile_error(),
    };

    out.into()
}
