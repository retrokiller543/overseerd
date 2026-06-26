//! `daemon!` expansion: assembles the daemon *and* validates it.
//!
//! ```ignore
//! let daemon = daemon! {
//!     name: "example-daemon",
//!     services: [Notifications, Echo],
//!     configs: [ DbConfig => "app.db.reader", DbConfig => "app.db.writer" ],
//!     managers: {
//!         // a pre-built manager instance ...
//!         directories: dirs,
//!         // ... or a per-manager config block the macro constructs + configures:
//!         config: { watch: true, sighup: true, debounce: std::time::Duration::from_millis(250) },
//!     },
//! }
//! .build()
//! .await?;
//! ```
//!
//! Each `managers` entry is either an **instance** (any expression) or a **config block**
//! (`{ key: value, .. }`) that applies settings to just that manager. A `config` block with
//! no `source` is loaded from the `directories` manager (which must then be present), so the
//! file-reload triggers (`sighup`/`watch`/`debounce`) configure the `ConfigManager` itself —
//! never the daemon. The listed `services` are asserted `Wired` (under `di-check`).

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Expr, Ident, LitBool, LitStr, Token, Type, braced, bracketed};

use crate::{di, paths::overseerd_path};

/// Parsed `daemon! { .. }`.
pub struct DaemonInput {
    name: Expr,
    services: Vec<Type>,
    components: Vec<Expr>,
    configs: Vec<ConfigEntry>,
    config_manager: Option<ManagerSource<ConfigSettings>>,
    directories_manager: Option<ManagerSource<DirSettings>>,
    middleware: Vec<Expr>,
    guards: Vec<Expr>,
    error_handler: Option<Expr>,
}

/// How a manager is supplied in the `managers` block: a pre-built instance, or settings the
/// macro uses to construct and configure it.
// A short-lived parse type built once per `daemon!`; variant size is irrelevant.
#[allow(clippy::large_enum_variant)]
enum ManagerSource<S> {
    Instance(Expr),
    Configure(S),
}

/// Settings for a macro-constructed `ConfigManager`.
#[derive(Default)]
struct ConfigSettings {
    /// A base manager expression to configure; if absent, the macro loads from the
    /// `directories` manager.
    source: Option<Expr>,
    /// Profiles passed to `load_from` when building a default source (`&[]` if omitted).
    profiles: Option<Expr>,
    sighup: bool,
    watch: bool,
    debounce: Option<Expr>,
}

impl Parse for ConfigSettings {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut settings = ConfigSettings::default();

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![:]>()?;

            match key.to_string().as_str() {
                "source" => settings.source = Some(input.parse()?),
                "profiles" => settings.profiles = Some(input.parse()?),
                "sighup" => settings.sighup = input.parse::<LitBool>()?.value,
                "watch" => settings.watch = input.parse::<LitBool>()?.value,
                "debounce" => settings.debounce = Some(input.parse()?),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown `config` setting `{other}`; expected `source`, `profiles`, \
                             `sighup`, `watch`, or `debounce`"
                        ),
                    ));
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(settings)
    }
}

/// Settings for a macro-constructed `DirectoriesManager`.
#[derive(Default)]
struct DirSettings {
    app: Option<Expr>,
    root: Option<Expr>,
}

impl Parse for DirSettings {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut settings = DirSettings::default();

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![:]>()?;

            match key.to_string().as_str() {
                "app" => settings.app = Some(input.parse()?),
                "root" => settings.root = Some(input.parse()?),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown `directories` setting `{other}`; expected `app` or `root`"),
                    ));
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(settings)
    }
}

/// Parses a manager value: a `{ .. }` config block, or any expression instance.
fn parse_manager_source<S: Parse>(input: ParseStream) -> syn::Result<ManagerSource<S>> {
    if input.peek(syn::token::Brace) {
        let content;
        braced!(content in input);

        Ok(ManagerSource::Configure(content.parse()?))
    } else {
        Ok(ManagerSource::Instance(input.parse()?))
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
        let mut config_manager = None;
        let mut directories_manager = None;
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
                "managers" => {
                    parse_managers(input, &mut config_manager, &mut directories_manager)?
                }
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
            config_manager,
            directories_manager,
            middleware,
            guards,
            error_handler,
        })
    }
}

/// Parses the `managers: { config: .., directories: .. }` block into the two optional
/// per-manager sources, rejecting duplicates and unknown keys.
fn parse_managers(
    input: ParseStream,
    config: &mut Option<ManagerSource<ConfigSettings>>,
    directories: &mut Option<ManagerSource<DirSettings>>,
) -> syn::Result<()> {
    let content;
    braced!(content in input);

    while !content.is_empty() {
        let key: Ident = content.parse()?;
        content.parse::<Token![:]>()?;

        match key.to_string().as_str() {
            "config" => {
                if config.is_some() {
                    return Err(syn::Error::new(key.span(), "duplicate `config` manager"));
                }

                *config = Some(parse_manager_source(&content)?);
            }

            "directories" => {
                if directories.is_some() {
                    return Err(syn::Error::new(key.span(), "duplicate `directories` manager"));
                }

                *directories = Some(parse_manager_source(&content)?);
            }

            other => {
                return Err(syn::Error::new(
                    key.span(),
                    format!("unknown manager `{other}`, expected `config` or `directories`"),
                ));
            }
        }

        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        }
    }

    Ok(())
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
        configs,
        config_manager,
        directories_manager,
        middleware,
        guards,
        error_handler,
    } = input;

    let config_tys = configs.iter().map(|entry| &entry.ty);
    let config_paths = configs.iter().map(|entry| &entry.path);

    let daemon = overseerd_path("Daemon");
    let config_manager_path = overseerd_path("ConfigManager");
    let directories_path = overseerd_path("DirectoriesManager");
    let config_dynamic = overseerd_path("config::Dynamic");

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

    // Materialize the directories manager (if any) once, so both the config loader and the
    // builder share the same instance.
    let mut directories_binding = quote!();
    let mut directories_call = quote!();
    let mut directories_available = false;

    match &directories_manager {
        Some(ManagerSource::Instance(expr)) => {
            directories_binding = quote!(let __overseerd_directories = #expr;);
            directories_call = quote!(.directories(__overseerd_directories));
            directories_available = true;
        }

        Some(ManagerSource::Configure(settings)) => {
            let expr = if let Some(root) = &settings.root {
                quote!(#directories_path::from_path(#root))
            } else if let Some(app) = &settings.app {
                quote!(#directories_path::for_app(#app))
            } else {
                return error("a `directories` config block needs `app` or `root`");
            };

            directories_binding = quote!(let __overseerd_directories = #expr;);
            directories_call = quote!(.directories(__overseerd_directories));
            directories_available = true;
        }

        None => {}
    }

    // Materialize the config manager (if any). A config block with no `source` is loaded
    // from the directories manager, so the trigger settings configure the manager itself.
    let mut config_binding = quote!();
    let mut config_call = quote!();

    match &config_manager {
        Some(ManagerSource::Instance(expr)) => {
            config_binding = quote!(let __overseerd_config = #expr;);
            config_call = quote!(.config_source(__overseerd_config));
        }

        Some(ManagerSource::Configure(settings)) => {
            let base = if let Some(source) = &settings.source {
                quote!(#source)
            } else if directories_available {
                let profiles = match &settings.profiles {
                    Some(profiles) => quote!(#profiles),
                    None => quote!(&[]),
                };

                quote!(#config_manager_path::<#config_dynamic>::load_from(&__overseerd_directories, #profiles)?)
            } else {
                return error(
                    "a `config` block without `source` requires a `directories` manager to load from",
                );
            };

            let mut chain = base;

            if settings.sighup {
                chain = quote!(#chain.reload_on_sighup());
            }

            if settings.watch {
                chain = quote!(#chain.watch_config());
            }

            if let Some(debounce) = &settings.debounce {
                chain = quote!(#chain.config_reload_debounce(#debounce));
            }

            config_binding = quote!(let __overseerd_config = #chain;);
            config_call = quote!(.config_source(__overseerd_config));
        }

        None => {}
    }

    let error_handler = error_handler.into_iter();

    quote! {
        {
            #assertion

            #directories_binding
            #config_binding

            #daemon::builder(#name)
                .auto_discover()
                #(.with_component(#components))*
                #(.config::<#config_tys>(#config_paths))*
                #config_call
                #directories_call
                #(.middleware(#middleware))*
                #(.guard(#guards))*
                #(.error_handler(#error_handler))*
        }
    }
}

/// A `compile_error!` expansion for an invalid `managers` configuration.
fn error(message: &str) -> TokenStream {
    syn::Error::new(Span::call_site(), message).to_compile_error()
}
