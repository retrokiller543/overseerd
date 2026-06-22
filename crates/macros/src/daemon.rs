//! `daemon!` expansion: assembles the daemon *and* validates it.
//!
//! ```ignore
//! let daemon = daemon! {
//!     name: "example-daemon",
//!     services: [Notifications, Echo],
//!     components: [Config { greeting: "Hi".into() }],
//! }
//! .build()
//! .await?;
//! ```
//!
//! Expands to a `DaemonBuilder`: `Daemon::builder(name).auto_discover()` plus a
//! `with_component(..)` for each listed instance. The listed `services` are also
//! asserted `Wired` (under `di-check`), so the same declaration that builds the
//! daemon validates its dependency graph at compile time — no separate list to
//! keep in sync.

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Expr, Ident, Token, Type, bracketed};

use crate::{di, paths::overseer_path};

/// Parsed `daemon! { name: .., services: [..], components: [..] }`.
pub struct DaemonInput {
    name: Expr,
    services: Vec<Type>,
    components: Vec<Expr>,
}

impl Parse for DaemonInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut services = Vec::new();
        let mut components = Vec::new();

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![:]>()?;

            match key.to_string().as_str() {
                "name" => name = Some(input.parse()?),
                "services" => services = bracketed_list::<Type>(input)?,
                "components" => components = bracketed_list::<Expr>(input)?,
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown `daemon!` key `{other}`, expected `name`, `services`, or `components`"),
                    ));
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        let name = name.ok_or_else(|| input.error("`daemon!` requires a `name`"))?;

        Ok(DaemonInput {
            name,
            services,
            components,
        })
    }
}

/// Parses `[a, b, c]` into a `Vec<T>`.
fn bracketed_list<T: Parse>(input: ParseStream) -> syn::Result<Vec<T>> {
    let content;
    bracketed!(content in input);
    let list = Punctuated::<T, Token![,]>::parse_terminated(&content)?;

    Ok(list.into_iter().collect())
}

pub fn expand(input: DaemonInput) -> TokenStream {
    let DaemonInput {
        name,
        services,
        components,
    } = input;

    let daemon = overseer_path("Daemon");

    // Under `di-check`, assert each listed service's whole graph is satisfied —
    // discharged here at the use site, where every `Provide` impl is visible.
    let assertion = if di::enabled() && !services.is_empty() {
        let wired = overseer_path("Wired");

        quote! {
            const _: () = {
                fn __overseer_assert_wired<T: #wired>() {}

                fn __overseer_daemon_check() {
                    #(__overseer_assert_wired::<#services>();)*
                }
            };
        }
    } else {
        quote!()
    };

    quote! {
        {
            #assertion

            #daemon::builder(#name)
                .auto_discover()
                #(.with_component(#components))*
        }
    }
}