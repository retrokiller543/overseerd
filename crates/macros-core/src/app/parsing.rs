use std::collections::HashSet;

use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, LitBool, Token, Type, braced, bracketed, parenthesized};

use super::model::{
    AppPhases, CliDeclarations, ConfigEntry, ConfigSettings, DirSettings, ManagerSource,
    PhaseArgument, PhaseInput,
};
use super::{AppAssembly, AppInput, NamedApp, command};

syn::custom_keyword!(app);

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

impl Parse for ConfigEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ty = input.parse()?;

        input.parse::<Token![=>]>()?;

        let path = input.parse()?;

        Ok(ConfigEntry { ty, path })
    }
}

impl Parse for AppInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(Token![#]) || input.peek(Token![pub]) || input.peek(app) {
            return Ok(Self::Named(input.parse()?));
        }

        Ok(Self::Legacy(AppAssembly::parse_with(input, false)?))
    }
}

impl Parse for NamedApp {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attributes = input.call(syn::Attribute::parse_outer)?;
        let visibility = input.parse()?;

        input.parse::<app>()?;

        let ident = input.parse()?;
        let content;

        braced!(content in input);

        let assembly = AppAssembly::parse_with(&content, true)?;

        if !input.is_empty() {
            return Err(input.error("unexpected tokens after named app definition"));
        }

        for attribute in &attributes {
            if !attribute.path().is_ident("doc") {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "only documentation attributes are supported on generated applications",
                ));
            }
        }

        Ok(Self {
            attributes,
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
        let mut cli = CliDeclarations::default();
        let mut keys = HashSet::new();

        while !input.is_empty() {
            let key = input.call(Ident::parse_any)?;
            let key_name = key.to_string();

            if reject_duplicates && !keys.insert(key_name.clone()) {
                return Err(syn::Error::new(
                    key.span(),
                    format!("duplicate app key `{key_name}`"),
                ));
            }

            if is_lifecycle_phase(&key_name) {
                if !reject_duplicates {
                    return Err(syn::Error::new(
                        key.span(),
                        "lifecycle phases require a named app definition",
                    ));
                }

                let phase = parse_phase(input, &key)?;

                set_phase(&mut phases, &key_name, phase);

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
                "components" => components = bracketed_list(input)?,
                "configs" => configs = bracketed_list(input)?,
                "managers" => parse_managers(input, &mut config_manager, &mut directories_manager)?,
                "middleware" => middleware = bracketed_list(input)?,
                "guards" => guards = bracketed_list(input)?,
                "error_handler" => error_handler = Some(input.parse()?),
                "overseerd" => overseerd = Some(input.parse()?),
                "crate" => krate = Some(input.parse()?),
                "args" | "commands" if !reject_duplicates => {
                    return Err(syn::Error::new(
                        key.span(),
                        "CLI declarations require a named app definition",
                    ));
                }
                "args" => cli.args = command::parse_args(input)?,
                "commands" => cli.commands = command::parse_commands(input)?,
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown `app!` key `{other}`, expected `name`, `protocol`, \
                             `services`, `components`, `configs`, `managers`, `middleware`, \
                             `guards`, `error_handler`, `args`, `commands`, `overseerd`, or `crate`"
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
            cli,
        })
    }
}

impl Parse for AppAssembly {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Self::parse_with(input, false)
    }
}

fn is_lifecycle_phase(key: &str) -> bool {
    matches!(
        key,
        "setup" | "configure" | "before_build" | "after_build" | "serve"
    )
}

fn set_phase(phases: &mut AppPhases, key: &str, phase: PhaseInput) {
    match key {
        "setup" => phases.setup = Some(phase),
        "configure" => phases.configure = Some(phase),
        "before_build" => phases.before_build = Some(phase),
        "after_build" => phases.after_build = Some(phase),
        "serve" => phases.serve = Some(phase),
        _ => unreachable!(),
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

        let arguments = Punctuated::<PhaseArgument, Token![,]>::parse_terminated(&arguments)?
            .into_iter()
            .collect::<Vec<_>>();
        let body = input.parse()?;
        let expected_arguments = if key == "setup" { 1 } else { 2 };

        if arguments.len() < expected_arguments
            || (key != "serve" && arguments.len() != expected_arguments)
        {
            return Err(syn::Error::new(
                key.span(),
                format!(
                    "`{key}` expects {expected_arguments} argument{}",
                    if expected_arguments == 1 { "" } else { "s" }
                ),
            ));
        }

        for argument in &arguments[..expected_arguments] {
            if argument.ty.is_some() {
                return Err(syn::Error::new(
                    argument.ident.span(),
                    "lifecycle context and app parameters cannot declare injected types",
                ));
            }
        }

        for argument in &arguments[expected_arguments..] {
            if argument.ty.is_none() {
                return Err(syn::Error::new(
                    argument.ident.span(),
                    "additional serve parameters require an injectable type",
                ));
            }
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

impl Parse for PhaseArgument {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident = input.call(Ident::parse_any)?;
        let ty = if input.peek(Token![:]) {
            input.parse::<Token![:]>()?;

            Some(input.parse()?)
        } else {
            None
        };

        Ok(Self { ident, ty })
    }
}

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

fn parse_manager_source<S: Parse>(input: ParseStream) -> syn::Result<ManagerSource<S>> {
    if input.peek(syn::token::Brace) {
        let content;

        braced!(content in input);

        return Ok(ManagerSource::Configure(content.parse()?));
    }

    Ok(ManagerSource::Instance(input.parse()?))
}

fn bracketed_list<T: Parse>(input: ParseStream) -> syn::Result<Vec<T>> {
    let content;

    bracketed!(content in input);

    let list = Punctuated::<T, Token![,]>::parse_terminated(&content)?;

    Ok(list.into_iter().collect())
}
