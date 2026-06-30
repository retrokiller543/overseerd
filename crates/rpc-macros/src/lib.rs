//! The Overseerd **RPC daemon** macros: `#[service]`, `#[handlers]`, `#[rpc]`, and
//! `app!`/`daemon!`. They are RPC-protocol-specific (they emit `::overseerd::daemon::*` types),
//! so they live in their own crate rather than the core `overseerd-macros`, built on the shared
//! [`overseerd_macros_core`] codegen.
//!
//! Re-exported through the `overseerd` facade's `daemon` module; depend on the facade, not this
//! crate directly.
//!
//! - `#[service]` is a **router component**: a `#[component]` (field-injected singleton) plus a
//!   service header, its `{Service}Rpcs` slice, and the generated client struct.
//! - `#[handlers]` is `MethodArgs<Rpcs>` — the base impl macro (`#[methods]`: `#[init]` +
//!   `#[hook]`) plus the RPC extension, which registers each `#[rpc]` method and contributes the
//!   client methods.
//! - `#[rpc]` marks a method inside a `#[handlers]` impl (a marker stripped by `#[handlers]`).
//!
//! `app!`/`daemon!` are **not** here — they are protocol-agnostic core macros (in
//! `overseerd-macros`), selecting a protocol via a required `protocol:` field.

extern crate proc_macro;

mod handlers;
mod router;
mod rpc;

use overseerd_macros_core::expand_component;
use overseerd_macros_core::methods::MethodArgs;
use overseerd_macros_core::paths::Paths;
use overseerd_macros_core::run;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use router::RouterComponent;
use syn::{ItemFn, ItemImpl, ItemStruct};

/// Declares a **service** — a router component exposing RPC methods.
///
/// `#[service]` is `ComponentArgs<Router>`: a `#[component]` (field-injected singleton) plus the
/// router surface — a `ServiceComponent` impl, the service's `{Service}Rpcs` slice and
/// `ServiceDescriptor`, and (under the `client` feature) the `{Service}Client<C>` struct. RPC
/// methods are added by `#[handlers]` impls. Accepts the component keys plus `version` /
/// `rpc_slice`.
#[proc_macro_attribute]
pub fn service(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = TokenStream2::from(attr);
    let out = match syn::parse2::<RouterComponent>(attr) {
        Ok(args) => {
            let paths = args.paths(Paths::overseerd_daemon());

            run::<ItemStruct, _>(item.into(), |item| expand_component(args, item, &paths))
        }

        Err(e) => e.into_compile_error(),
    };

    out.into()
}

/// Contributes the `#[rpc]` methods (and an optional `#[init]` / `#[hook]`s) of an inherent
/// `impl` block to the service of `Self`.
///
/// `#[handlers]` is `#[methods]` plus RPC registration: it claims each `#[rpc]` method into the
/// service's `{Service}Rpcs` slice and contributes the client methods, while the shared base
/// also handles `#[init]` constructors and `#[hook]` methods. Several `#[handlers]` blocks for
/// one service merge with no coordination. Accepts `rpc_slice = ..` and (legacy) `client_trait
/// = ..`.
#[proc_macro_attribute]
pub fn handlers(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = TokenStream2::from(attr);
    let out = match syn::parse2::<MethodArgs<handlers::Rpcs>>(attr) {
        Ok(args) => {
            let paths = args.paths(Paths::overseerd_daemon());

            run::<ItemImpl, _>(item.into(), |item| {
                overseerd_macros_core::methods::expand(args, item, &paths)
            })
        }

        Err(e) => e.into_compile_error(),
    };

    out.into()
}

/// Marks a method inside a `#[handlers]` impl as an RPC. A **marker** consumed and stripped by
/// `#[handlers]`; used on its own it emits a `compile_error!`.
#[proc_macro_attribute]
pub fn rpc(_attr: TokenStream, item: TokenStream) -> TokenStream {
    run::<ItemFn, _>(item.into(), rpc::expand_standalone).into()
}
