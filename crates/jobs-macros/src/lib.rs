//! The Overseerd **job** macros: `#[jobs]` and `#[job]`. They emit `::overseerd::jobs::*`
//! types, so — like the RPC and axum protocol macros — they live in their own crate built on
//! the shared [`overseerd_macros_core`] codegen, rather than in the core `overseerd-macros`.
//!
//! Re-exported through the `overseerd` facade's `jobs` module (with the `jobs` feature); depend
//! on the facade, not this crate directly.
//!
//! - `#[jobs]` is `MethodArgs<Jobs>` — the base impl macro (`#[methods]`: `#[init]` + `#[hook]`)
//!   plus the jobs extension, which registers each `#[job]` method into the global `JOBS` slice.
//!   A type that is both a service and has jobs uses two impl blocks — one `#[handlers]`, one
//!   `#[jobs]` — so no protocol macro ever needs to know about jobs.
//! - `#[job]` marks a method inside a `#[jobs]` impl (a marker stripped by `#[jobs]`).

extern crate proc_macro;

mod jobs;

use jobs::Jobs;
use overseerd_macros_core::methods::MethodArgs;
use overseerd_macros_core::paths::Paths;
use overseerd_macros_core::run;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use syn::{ItemFn, ItemImpl};

/// The default crate roots for the job macros. Core is always the `overseerd` facade; the
/// plugin (own-types) root is `::overseerd::jobs` when consumed through the facade (the
/// `facade` feature, set by the `overseerd` crate) and the standalone `::overseerd_jobs`
/// otherwise — so a direct dependant on `overseerd-jobs` gets working codegen.
fn jobs_paths() -> Paths {
    if cfg!(feature = "facade") {
        Paths::new(
            syn::parse_quote!(::overseerd),
            syn::parse_quote!(::overseerd::jobs),
        )
    } else {
        Paths::new(
            syn::parse_quote!(::overseerd),
            syn::parse_quote!(::overseerd_jobs),
        )
    }
}

/// Registers the `#[job]` methods (and an optional `#[init]` / `#[hook]`s) of an inherent
/// `impl` block as scheduled jobs.
///
/// `#[jobs]` is `#[methods]` plus job registration: it claims each `#[job]` method into the
/// global `JOBS` slice, while the shared base also handles `#[init]` constructors and `#[hook]`
/// methods. Several `#[jobs]` blocks (and `#[handlers]` blocks) for one type merge with no
/// coordination.
#[proc_macro_attribute]
pub fn jobs(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = TokenStream2::from(attr);
    let out = match syn::parse2::<MethodArgs<Jobs>>(attr) {
        Ok(args) => {
            let paths = args.paths(jobs_paths());

            run::<ItemImpl, _>(item.into(), |item| {
                overseerd_macros_core::methods::expand(args, item, &paths)
            })
        }

        Err(e) => e.into_compile_error(),
    };

    out.into()
}

/// Marks a method inside a `#[jobs]` impl as a scheduled job. A **marker** consumed and stripped
/// by `#[jobs]`; used on its own it emits a `compile_error!`.
#[proc_macro_attribute]
pub fn job(_attr: TokenStream, item: TokenStream) -> TokenStream {
    run::<ItemFn, _>(item.into(), |_| {
        Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[job] is only valid on a method inside a `#[jobs]` impl block",
        ))
    })
    .into()
}
