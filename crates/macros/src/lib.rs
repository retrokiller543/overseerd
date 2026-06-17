//! Procedural macros for Overseer: `#[service]`, `#[handlers]`, `#[rpc]`.
//!
//! - `#[service(id = "...", version = "...")]` on a **struct** declares a
//!   service's identity, tied to that type, and (by default) a field-injection
//!   singleton factory.
//! - `#[handlers]` on an **impl** block contributes its `#[rpc]` methods to the
//!   service of `Self`, and turns an optional `#[init]` constructor into an
//!   explicit singleton factory that overrides the default. Several `#[handlers]`
//!   impls may target one service.
//! - `#[rpc]` / `#[init]` are markers consumed by `#[handlers]`.
//!
//! Structure follows the dtolnay convention (see `thiserror-impl`): thin
//! `#[proc_macro_attribute]` entry points delegating to `expand` functions that
//! return `syn::Result`, with errors surfaced via `into_compile_error`.

extern crate proc_macro;

mod attr;
mod derive;
mod handlers;
mod rpc;
mod service;

use proc_macro::TokenStream;
use syn::{DeriveInput, ItemFn, ItemImpl, ItemStruct, parse_macro_input};

/// Implements the `Component` metadata trait for a plain dependency type, so it
/// can be registered via `DaemonBuilder::with_component`.
#[proc_macro_derive(Component, attributes(component))]
pub fn derive_component(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);

    derive::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Declares a service's identity on its type (and a default singleton factory).
#[proc_macro_attribute]
pub fn service(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as attr::ServiceArgs);
    let item = parse_macro_input!(item as ItemStruct);

    service::expand(args, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Contributes the `#[rpc]` methods (and optional `#[init]`) of an impl block to
/// the service of `Self`.
#[proc_macro_attribute]
pub fn handlers(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item = parse_macro_input!(item as ItemImpl);

    handlers::expand(item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Marks a method inside a `#[handlers]` impl as an RPC.
///
/// Consumed (and stripped) by `#[handlers]`. Reaching its own expansion means it
/// was used outside a `#[handlers]` block, which is an error.
#[proc_macro_attribute]
pub fn rpc(attr: TokenStream, item: TokenStream) -> TokenStream {
    let _ = parse_macro_input!(attr as attr::RpcArgs);
    let item = parse_macro_input!(item as ItemFn);

    rpc::expand_standalone(item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
