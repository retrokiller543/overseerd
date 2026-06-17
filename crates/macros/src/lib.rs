//! Procedural macros for Overseer: `#[service]` and `#[rpc]`.
//!
//! `#[service]` annotates an inherent `impl` block and turns each `#[rpc]`
//! method into a registered `RpcDescriptor`, emitting a `ServiceDescriptor`
//! collected via `inventory`. `#[rpc]` is a marker consumed by `#[service]`;
//! used on its own it produces a friendly compile error.
//!
//! Structure follows the dtolnay convention (see `thiserror-impl`): thin
//! `#[proc_macro_attribute]` entry points that delegate to `expand` functions
//! returning `syn::Result`, with errors surfaced through
//! `syn::Error::into_compile_error` rather than panics.

extern crate proc_macro;

mod attr;
mod rpc;
mod service;

use proc_macro::TokenStream;
use syn::{ItemFn, ItemImpl, parse_macro_input};

/// Registers each `#[rpc]` method of an inherent `impl` block as a service RPC.
///
/// ```ignore
/// #[service(id = "greeter", version = "0.1")]
/// impl Greeter {
///     #[rpc]
///     async fn ping() -> overseer_core::Result<Pong> { /* ... */ }
/// }
/// ```
#[proc_macro_attribute]
pub fn service(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as attr::ServiceArgs);
    let item = parse_macro_input!(item as ItemImpl);

    service::expand(args, item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Marks a method inside a `#[service]` impl as an RPC.
///
/// This attribute is consumed (and stripped) by `#[service]`. Reaching its own
/// expansion means it was used outside a `#[service]` block, which is an error.
#[proc_macro_attribute]
pub fn rpc(attr: TokenStream, item: TokenStream) -> TokenStream {
    let _ = parse_macro_input!(attr as attr::RpcArgs);
    let item = parse_macro_input!(item as ItemFn);

    rpc::expand_standalone(item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
