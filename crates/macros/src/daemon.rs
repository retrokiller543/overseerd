//! `daemon!` expansion: assembles the daemon *and* validates it.
//!
//! ```ignore
//! let daemon = daemon! {
//!     name: "example-daemon",
//!     services: [Notifications, Echo],
//!     configs: [ DbConfig => "app.db.reader", DbConfig => "app.db.writer" ],
//!     managers: {
//!         config: config,        // a pre-built `ConfigManager`
//!         directories: dirs,     // a pre-built `DirectoriesManager`
//!     },
//! }
//! .build()
//! .await?;
//! ```
//!
//! Expands to a `DaemonBuilder`: `Daemon::builder(name).auto_discover()`, a
//! `with_component(..)` for each listed instance, a `config::<T>(path)` for each
//! `configs` entry, and `config_source`/`directories` calls for the `managers`
//! bindings (both optional — the builder constructs defaults otherwise). The listed
//! `services` are also asserted `Wired` (under `di-check`), so the same declaration
//! that builds the daemon validates its dependency graph at compile time — no
//! separate list to keep in sync.

use proc_macro2::TokenStream;
use quote::quote;
use std::collections::HashSet;
use std::hash::Hash;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Expr, Ident, LitStr, Token, Type, braced, bracketed};

use crate::{di, paths::overseerd_path};

/// Parsed `daemon! { name: .., services: [..], components: [..], configs: [..] }`.
pub struct DaemonInput {
    name: Expr,
    services: Vec<Type>,
    components: Vec<Expr>,
    configs: Vec<ConfigEntry>,
    managers: HashSet<ManagerInstance>,
    middleware: Vec<Expr>,
    guards: Vec<Expr>,
    error_handler: Option<Expr>,
}

/// Parses <manager>: <ident>
pub enum ManagerInstance {
    Config(Option<Ident>),
    Directories(Option<Ident>),
}

impl PartialEq for ManagerInstance {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Config(_), Self::Config(_)) | (Self::Directories(_), Self::Directories(_))
        )
    }
}

impl Eq for ManagerInstance {}

impl Hash for ManagerInstance {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Config(_) => "config".hash(state),
            Self::Directories(_) => "directories".hash(state),
        }
    }
}

impl Parse for ManagerInstance {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident: Ident = input.parse()?;
        input.parse::<Token![:]>()?;

        match ident.to_string().as_str() {
            "config" => Ok(Self::Config(Some(input.parse()?))),
            "directories" => Ok(Self::Directories(Some(input.parse()?))),
            _ => Err(syn::Error::new(input.span(), "Unknown manager instance")),
        }
    }
}

impl quote::ToTokens for ManagerInstance {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let default_config = Ident::new("config", proc_macro2::Span::call_site());
        let default_directories = Ident::new("directories", proc_macro2::Span::call_site());

        let ident = match self {
            ManagerInstance::Config(ident) => ident.as_ref().unwrap_or(&default_config),
            ManagerInstance::Directories(ident) => ident.as_ref().unwrap_or(&default_directories),
        };

        let new_tokens = quote! {
            #ident
        };

        tokens.extend(new_tokens);
    }
}

/// One `configs:` entry — `Type => "property.path"`.
struct ConfigEntry {
    ty: Type,
    path: LitStr,
}

impl Parse for ConfigEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ty: Type = input.parse()?;
        input.parse::<Token![=>]>()?;
        let path: LitStr = input.parse()?;

        Ok(ConfigEntry { ty, path })
    }
}

impl Parse for DaemonInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name = None;
        let mut services = Vec::new();
        let mut components = Vec::new();
        let mut configs = Vec::new();
        let mut managers = HashSet::new();
        let mut middleware = Vec::new();
        let mut guards = Vec::new();
        let mut error_handler = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![:]>()?;

            match key.to_string().as_str() {
                "name" => name = Some(input.parse()?),
                "services" => services = bracketed_list::<Type>(input)?,
                "components" => components = bracketed_list::<Expr>(input)?,
                "configs" => configs = bracketed_list::<ConfigEntry>(input)?,
                "managers" => managers = braced::<ManagerInstance>(input)?.collect(),
                "middleware" => middleware = bracketed_list::<Expr>(input)?,
                "guards" => guards = bracketed_list::<Expr>(input)?,
                "error_handler" => error_handler = Some(input.parse()?),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown `daemon!` key `{other}`, expected `name`, `services`, \
                             `components`, `configs`, `managers`, `middleware`, `guards`, or \
                             `error_handler`"
                        ),
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
            configs,
            managers,
            middleware,
            guards,
            error_handler,
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

fn braced<T: Parse>(input: ParseStream) -> syn::Result<impl Iterator<Item = T>> {
    let content;
    braced!(content in input);
    let list = Punctuated::<T, Token![,]>::parse_terminated(&content)?;

    Ok(list.into_iter())
}

pub fn expand(input: DaemonInput) -> TokenStream {
    let DaemonInput {
        name,
        services,
        components,
        configs,
        managers,
        middleware,
        guards,
        error_handler,
    } = input;

    let config_tys = configs.iter().map(|entry| &entry.ty);
    let config_paths = configs.iter().map(|entry| &entry.path);

    let daemon = overseerd_path("Daemon");

    // Under `di-check`, assert each listed service's whole graph is satisfied —
    // discharged here at the use site, where every `Provide` impl is visible.
    let assertion = if di::enabled() && !services.is_empty() {
        let wired = overseerd_path("Wired");

        quote! {
            const _: () = {
                fn __overseerd_assert_wired<T: #wired>() {}

                fn __overseerd_daemon_check() {
                    #(__overseerd_assert_wired::<#services>();)*
                }
            };
        }
    } else {
        quote!()
    };

    let config_manager = managers.get(&ManagerInstance::Config(None)).into_iter();
    let directories_manager = managers
        .get(&ManagerInstance::Directories(None))
        .into_iter();

    let error_handler = error_handler.into_iter();

    quote! {
        {
            #assertion

            #daemon::builder(#name)
                .auto_discover()
                #(.with_component(#components))*
                #(.config::<#config_tys>(#config_paths))*
                #(.config_source(#config_manager))*
                #(.directories(#directories_manager))*
                #(.middleware(#middleware))*
                #(.guard(#guards))*
                #(.error_handler(#error_handler))*
        }
    }
}
