//! `app!` expansion: defines a reusable application host or assembles a legacy builder.
//!
//! ```ignore
//! app! {
//!     pub app Example {
//!         name: "example-daemon",
//!         protocol: RpcPlugin,
//!         services: [Notifications, Echo],
//!         configs: [DbConfig => "app.db.reader", DbConfig => "app.db.writer"],
//!     }
//! }
//!
//! let app = Example::builder()?.build().await?;
//! ```
//!
//! Each `managers` entry is either an **instance** (any expression) or a **config block**
//! (`{ key: value, .. }`) that applies settings to just that manager. A `config` block with
//! no `source` is loaded from the `directories` manager (which must then be present), so the
//! file-reload triggers (`sighup`/`watch`/`debounce`) configure the `ConfigManager` itself —
//! never the app. The listed `services` are asserted `Wired` (under `di-check`).

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{
    Block, Expr, Ident, LitBool, LitStr, Path, Token, Type, Visibility, braced, bracketed,
    parenthesized,
};

use crate::{di, paths::Paths};

syn::custom_keyword!(app);

/// Parsed input accepted by `app!`.
pub(crate) enum AppInput {
    /// A reusable named application definition.
    Named(NamedApp),
    /// The temporary expression-oriented application builder form.
    Legacy(AppAssembly),
}

/// A reusable named application definition.
pub(crate) struct NamedApp {
    visibility: Visibility,
    ident: Ident,
    assembly: AppAssembly,
}

/// The protocol-specific builder assembly shared by both macro forms.
pub(crate) struct AppAssembly {
    name: Expr,
    /// The protocol plugin type `P` the app installs (`protocol: SomeProtocolPlugin`). Required
    /// — `app!` is protocol-agnostic, so the protocol must be named.
    protocol: Type,
    services: Vec<Type>,
    components: Vec<Expr>,
    configs: Vec<ConfigEntry>,
    config_manager: Option<ManagerSource<ConfigSettings>>,
    directories_manager: Option<ManagerSource<DirSettings>>,
    middleware: Vec<Expr>,
    guards: Vec<Expr>,
    error_handler: Option<Expr>,
    /// Override for the core `overseerd` facade root (`overseerd: ::path`).
    overseerd: Option<syn::Path>,
    /// Override for the plugin own-types root (`crate: ::path`).
    krate: Option<syn::Path>,
    phases: AppPhases,
}

#[derive(Default)]
struct AppPhases {
    setup: Option<PhaseInput>,
    configure: Option<PhaseInput>,
    before_build: Option<PhaseInput>,
    after_build: Option<PhaseInput>,
    serve: Option<PhaseInput>,
}

enum PhaseInput {
    Path(Path),
    Inline { arguments: Vec<Ident>, body: Block },
}

/// How a manager is supplied in the `managers` block: a pre-built instance, or settings the
/// macro uses to construct and configure it.
// A short-lived parse type built once per `app!`; variant size is irrelevant.
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
            let key = input.call(Ident::parse_any)?;
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
            let key = input.call(Ident::parse_any)?;
            input.parse::<Token![:]>()?;

            match key.to_string().as_str() {
                "app" => settings.app = Some(input.parse()?),
                "root" => settings.root = Some(input.parse()?),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown `directories` setting `{other}`; expected `app` or `root`"
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

impl Parse for AppInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(Token![pub]) || input.peek(app) {
            return Ok(Self::Named(input.parse()?));
        }

        Ok(Self::Legacy(AppAssembly::parse_with(input, false)?))
    }
}

impl Parse for NamedApp {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let visibility = input.parse()?;
        input.parse::<app>()?;
        let ident = input.parse()?;

        let content;
        braced!(content in input);
        let assembly = AppAssembly::parse_with(&content, true)?;

        if !input.is_empty() {
            return Err(input.error("unexpected tokens after named app definition"));
        }

        Ok(Self {
            visibility,
            ident,
            assembly,
        })
    }
}

impl AppAssembly {
    fn parse_with(input: ParseStream, reject_duplicates: bool) -> syn::Result<Self> {
        let mut name = None;
        let mut protocol = None;
        let mut services = Vec::new();
        let mut components = Vec::new();
        let mut configs = Vec::new();
        let mut config_manager = None;
        let mut directories_manager = None;
        let mut middleware = Vec::new();
        let mut guards = Vec::new();
        let mut error_handler = None;
        let mut overseerd = None;
        let mut krate = None;
        let mut phases = AppPhases::default();
        let mut keys = std::collections::HashSet::new();

        while !input.is_empty() {
            let key = input.call(Ident::parse_any)?;
            let key_name = key.to_string();

            if reject_duplicates && !keys.insert(key_name.clone()) {
                return Err(syn::Error::new(
                    key.span(),
                    format!("duplicate app key `{key_name}`"),
                ));
            }

            if matches!(
                key_name.as_str(),
                "setup" | "configure" | "before_build" | "after_build" | "serve"
            ) {
                if !reject_duplicates {
                    return Err(syn::Error::new(
                        key.span(),
                        "lifecycle phases require a named app definition",
                    ));
                }

                let phase = parse_phase(input, &key)?;

                match key_name.as_str() {
                    "setup" => phases.setup = Some(phase),
                    "configure" => phases.configure = Some(phase),
                    "before_build" => phases.before_build = Some(phase),
                    "after_build" => phases.after_build = Some(phase),
                    "serve" => phases.serve = Some(phase),
                    _ => unreachable!(),
                }

                if input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                }

                continue;
            }

            input.parse::<Token![:]>()?;

            match key_name.as_str() {
                "name" => name = Some(input.parse()?),
                "protocol" => protocol = Some(input.parse()?),
                "services" => services = bracketed_list::<Type>(input)?,
                "components" => components = bracketed_list::<Expr>(input)?,
                "configs" => configs = bracketed_list::<ConfigEntry>(input)?,
                "managers" => parse_managers(input, &mut config_manager, &mut directories_manager)?,
                "middleware" => middleware = bracketed_list::<Expr>(input)?,
                "guards" => guards = bracketed_list::<Expr>(input)?,
                "error_handler" => error_handler = Some(input.parse()?),
                "overseerd" => overseerd = Some(input.parse()?),
                "crate" => krate = Some(input.parse()?),
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown `app!` key `{other}`, expected `name`, `protocol`, \
                             `services`, `components`, `configs`, `managers`, `middleware`, \
                             `guards`, `error_handler`, `overseerd`, or `crate`"
                        ),
                    ));
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        let name = name.ok_or_else(|| input.error("`app!` requires a `name`"))?;
        let protocol = protocol.ok_or_else(|| {
            input.error("`app!` requires a `protocol: <ProtocolPlugin>` (e.g. the RPC daemon's)")
        })?;

        Ok(Self {
            name,
            protocol,
            services,
            components,
            configs,
            config_manager,
            directories_manager,
            middleware,
            guards,
            error_handler,
            overseerd,
            krate,
            phases,
        })
    }
}

fn parse_phase(input: ParseStream, key: &Ident) -> syn::Result<PhaseInput> {
    if input.peek(Token![=]) {
        input.parse::<Token![=]>()?;

        return Ok(PhaseInput::Path(input.parse()?));
    }

    if input.peek(syn::token::Paren) {
        let arguments;
        parenthesized!(arguments in input);
        let arguments = Punctuated::<Ident, Token![,]>::parse_terminated(&arguments)?
            .into_iter()
            .collect::<Vec<_>>();
        let body = input.parse()?;
        let expected_arguments = if key == "setup" { 1 } else { 2 };

        if arguments.len() != expected_arguments {
            return Err(syn::Error::new(
                key.span(),
                format!(
                    "`{key}` expects {expected_arguments} argument{}",
                    if expected_arguments == 1 { "" } else { "s" }
                ),
            ));
        }

        return Ok(PhaseInput::Inline { arguments, body });
    }

    if input.peek(Token![:]) {
        return Err(syn::Error::new(
            key.span(),
            "declarative lifecycle settings are reserved for the generated CLI bootstrap; use `phase = async_function` or `phase(args...) { ... }`",
        ));
    }

    Err(syn::Error::new(
        key.span(),
        "expected `= async_function` or `(arguments...) { ... }` after lifecycle phase",
    ))
}

impl Parse for AppAssembly {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Self::parse_with(input, false)
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
                    return Err(syn::Error::new(
                        key.span(),
                        "duplicate `directories` manager",
                    ));
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

pub fn expand(input: AppInput) -> TokenStream {
    match input {
        AppInput::Named(input) => expand_named(input),
        AppInput::Legacy(input) => expand_builder(input),
    }
}

fn expand_named(input: NamedApp) -> TokenStream {
    let NamedApp {
        visibility,
        ident,
        mut assembly,
    } = input;
    let phases = std::mem::take(&mut assembly.phases);
    let protocol = assembly.protocol.clone();
    let paths = Paths::overseerd().resolve(assembly.overseerd.clone(), assembly.krate.clone());
    let app_builder = paths.core("AppBuilder");
    let config_error = paths.core("ConfigError");
    let app = paths.core("App");
    let app_host = paths.core("AppHost");
    let bootstrap_context = paths.core("BootstrapContext");
    let execution_mode = paths.core("ExecutionMode");
    let host_error = paths.core("HostError");
    let lifecycle_phase = paths.core("LifecyclePhase");
    let phase_error = paths.core("PhaseError");
    let prepared_app = paths.core("PreparedApp");
    let builder = expand_builder(assembly);
    let setup_call = phase_result(
        phases.setup.as_ref(),
        quote!(context),
        &[quote!(context)],
        quote!(#lifecycle_phase::Setup),
        &phase_error,
    );
    let configure_call = phase_call(
        phases.configure.as_ref(),
        quote!(builder),
        &[quote!(&mut context), quote!(builder)],
        quote!(#lifecycle_phase::Configure),
        &phase_error,
    );
    let before_build_call = phase_call(
        phases.before_build.as_ref(),
        quote!(builder),
        &[quote!(&mut context), quote!(builder)],
        quote!(#lifecycle_phase::BeforeBuild),
        &phase_error,
    );
    let after_build_call = phase_call(
        phases.after_build.as_ref(),
        quote!(app),
        &[quote!(&mut context), quote!(app)],
        quote!(#lifecycle_phase::AfterBuild),
        &phase_error,
    );
    let serve_method = phases.serve.as_ref().map(|serve| {
        let serve_call = phase_call(
            Some(serve),
            quote!(()),
            &[quote!(context), quote!(app)],
            quote!(#lifecycle_phase::Serve),
            &phase_error,
        );

        quote! {
            /// Runs the application-defined serve lifecycle phase.
            pub async fn serve_with(
                context: #bootstrap_context,
                app: #app<#protocol>,
            ) -> ::core::result::Result<(), #phase_error> {
                let output = #serve_call;

                Ok(output)
            }
        }
    });

    quote! {
        #[doc = "Generated application host."]
        #visibility struct #ident;

        impl #ident {
            /// Creates a new configured application builder.
            pub fn builder() -> ::core::result::Result<#app_builder<#protocol>, #config_error> {
                ::core::result::Result::Ok(#builder)
            }

            /// Creates the lifecycle bootstrap context.
            pub async fn setup(mode: #execution_mode) -> ::core::result::Result<#bootstrap_context, #phase_error> {
                let context = #bootstrap_context::new(mode);

                Self::__overseerd_setup_context(context).await
            }

            async fn __overseerd_setup_context(
                context: #bootstrap_context,
            ) -> ::core::result::Result<#bootstrap_context, #phase_error> {
                #setup_call
            }

            /// Configures and validates the app without constructing ordinary components.
            pub async fn prepare(
                mode: #execution_mode,
            ) -> ::core::result::Result<(#bootstrap_context, #prepared_app<#protocol>), #phase_error> {
                let mut context = Self::setup(mode).await?;
                let builder = Self::builder()
                    .map_err(|source| #phase_error::new(#lifecycle_phase::Configure, source))?;
                let builder = #configure_call;
                let builder = #before_build_call;
                let prepared = builder
                    .prepare()
                    .map_err(|source| #phase_error::new(#lifecycle_phase::Prepare, source))?;

                Ok((context, prepared))
            }

            /// Runs the host lifecycle through component and protocol construction.
            pub async fn build(
                mode: #execution_mode,
            ) -> ::core::result::Result<(#bootstrap_context, #app<#protocol>), #phase_error> {
                if mode.is_tooling() {
                    return Err(#phase_error::new(
                        #lifecycle_phase::Build,
                        #host_error::ToolingConstruction,
                    ));
                }

                let (mut context, prepared) = Self::prepare(mode).await?;
                let app = prepared
                    .build()
                    .await
                    .map_err(|source| #phase_error::new(#lifecycle_phase::Build, source))?;
                let app = #after_build_call;

                Ok((context, app))
            }

            #serve_method
        }

        impl #app_host for #ident {
            type Protocol = #protocol;

            fn builder() -> ::core::result::Result<#app_builder<#protocol>, #config_error> {
                Self::builder()
            }
        }
    }
}

fn phase_call(
    phase: Option<&PhaseInput>,
    default: TokenStream,
    values: &[TokenStream],
    lifecycle_phase: TokenStream,
    phase_error: &Path,
) -> TokenStream {
    match phase {
        Some(PhaseInput::Path(path)) => quote! {
            #path(#(#values),*)
                .await
                .map_err(|source| #phase_error::new(#lifecycle_phase, source))?
        },
        Some(PhaseInput::Inline { arguments, body }) => quote! {
            {
                let (#(#arguments,)*) = (#(#values,)*);

                (async move #body)
                    .await
                    .map_err(|source| #phase_error::new(#lifecycle_phase, source))?
            }
        },
        None => default,
    }
}

fn phase_result(
    phase: Option<&PhaseInput>,
    default: TokenStream,
    values: &[TokenStream],
    lifecycle_phase: TokenStream,
    phase_error: &Path,
) -> TokenStream {
    match phase {
        Some(PhaseInput::Path(path)) => quote! {
            #path(#(#values),*)
                .await
                .map_err(|source| #phase_error::new(#lifecycle_phase, source))
        },
        Some(PhaseInput::Inline { arguments, body }) => quote! {
            {
                let (#(#arguments,)*) = (#(#values,)*);

                (async move #body)
                    .await
                    .map_err(|source| #phase_error::new(#lifecycle_phase, source))
            }
        },
        None => quote!(Ok(#default)),
    }
}

fn expand_builder(input: AppAssembly) -> TokenStream {
    let AppAssembly {
        name,
        protocol,
        services,
        components,
        configs,
        config_manager,
        directories_manager,
        middleware,
        guards,
        error_handler,
        overseerd,
        krate,
        phases: _,
    } = input;

    // `app!` is a core macro; its emitted items are all core (`App`, `ConfigManager`, …),
    // resolved against the `overseerd` facade unless overridden per-invocation.
    let paths = &Paths::overseerd().resolve(overseerd, krate);

    let config_tys = configs.iter().map(|entry| &entry.ty);
    let config_paths = configs.iter().map(|entry| &entry.path);

    // The protocol-agnostic core `App`, specialized to the chosen protocol plugin.
    let app_ty = paths.core("App");
    let config_manager_path = paths.core("ConfigManager");
    let directories_path = paths.core("DirectoriesManager");
    let config_dynamic = paths.core("config::Dynamic");

    // Under `di-check`, assert each listed service's whole graph is satisfied —
    // discharged here at the use site, where every `Provide` impl is visible.
    let assertion = if di::enabled() && !services.is_empty() {
        let wired = paths.core("Wired");

        quote! {
            const _: () = {
                fn __overseerd_assert_wired<T: #wired>() {}

                fn __overseerd_app_check() {
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

    // `middleware`/`guard`/`error_handler` are protocol-builder methods (e.g. the RPC daemon's
    // `RpcAppBuilder`). `app!` is protocol-agnostic, so it does not import any specific builder
    // trait — the caller brings their protocol's builder extension into scope (its prelude).
    let error_handler = error_handler.into_iter();

    quote! {
        {
            #assertion

            #directories_binding
            #config_binding

            #app_ty::<#protocol>::builder(#name)
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

#[cfg(test)]
mod tests;
