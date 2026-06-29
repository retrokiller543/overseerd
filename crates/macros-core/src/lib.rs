//! Shared codegen library for the Overseerd proc-macros.
//!
//! A proc-macro crate can only export proc-macros, so all the reusable machinery — the
//! attribute parsing, the component / service / config / hook / handler expansions, the
//! client generation, and the crate-path resolution — lives here as an ordinary library.
//! `overseerd-macros` (and each plugin's macro crate) is a thin set of `#[proc_macro_*]`
//! shims that forward to the `expand_*` entry points below.
//!
//! The seams for extending the macros — [`Paths`] (crate-root resolution) and [`ParseKeyed`]
//! (attribute-argument extension) — are exposed for plugin macro crates to build on.

mod app;
mod attr;
mod case;
mod component;
mod config;
mod di;
mod handle;
mod handlers;
mod hook;
mod inject;
mod injectable;
mod methods;
mod parse_keyed;
mod paths;
mod provide;
mod rpc;
mod service;

pub use parse_keyed::{NoExt, ParseKeyed, eat_comma, eat_eq, unknown_key_error};
pub use paths::Paths;

use proc_macro2::TokenStream;
use syn::{DeriveInput, ItemFn, ItemImpl, ItemStruct, ItemTrait};

/// Parses `item` as `T` and runs `expand`, turning a parse or expansion error into a
/// `compile_error!` token stream so the macro never panics.
fn run<T, F>(item: TokenStream, expand: F) -> TokenStream
where
    T: syn::parse::Parse,
    F: FnOnce(T) -> syn::Result<TokenStream>,
{
    match syn::parse2::<T>(item) {
        Ok(parsed) => expand(parsed).unwrap_or_else(syn::Error::into_compile_error),

        Err(e) => e.into_compile_error(),
    }
}

/// `#[component]` expansion entry point.
pub fn component(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = match syn::parse2::<attr::ServiceArgs>(attr) {
        Ok(args) => args,

        Err(e) => return e.into_compile_error(),
    };

    run::<ItemStruct, _>(item, |item| component::expand(args, item))
}

/// `#[config]` expansion entry point.
pub fn config(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = match syn::parse2::<config::ConfigArgs>(attr) {
        Ok(args) => args,

        Err(e) => return e.into_compile_error(),
    };

    run::<DeriveInput, _>(item, |item| config::expand(args, item))
}

/// `#[service]` expansion entry point.
pub fn service(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = match syn::parse2::<attr::ServiceArgs>(attr) {
        Ok(args) => args,

        Err(e) => return e.into_compile_error(),
    };

    run::<ItemStruct, _>(item, |item| service::expand(args, item))
}

/// `#[handlers]` expansion entry point.
pub fn handlers(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = match syn::parse2::<attr::HandlersArgs>(attr) {
        Ok(args) => args,

        Err(e) => return e.into_compile_error(),
    };

    run::<ItemImpl, _>(item, |item| handlers::expand(args, item))
}

/// `#[methods]` expansion entry point.
pub fn methods(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = match syn::parse2::<attr::MethodsArgs>(attr) {
        Ok(args) => args,

        Err(e) => return e.into_compile_error(),
    };

    run::<ItemImpl, _>(item, |item| methods::expand(args, item))
}

/// `#[rpc]` (standalone) expansion entry point.
pub fn rpc(item: TokenStream) -> TokenStream {
    run::<ItemFn, _>(item, rpc::expand_standalone)
}

/// `#[injectable]` expansion entry point.
pub fn injectable(item: TokenStream) -> TokenStream {
    run::<ItemTrait, _>(item, |item| Ok(injectable::expand(item)))
}

/// `app!` / `daemon!` expansion entry point.
pub fn app(input: TokenStream) -> TokenStream {
    run::<app::AppInput, _>(input, |input| Ok(app::expand(input)))
}
