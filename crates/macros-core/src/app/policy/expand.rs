use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Ident, LitChar, LitStr, Visibility};

use super::{ArgumentDefault, ArgumentPolicy, CliPolicy, ServePolicy};
use crate::app::model::{Declared, PhaseInput};
use crate::paths::Paths;

/// Generated parser, bootstrap conversion, and serve command shape.
#[cfg_attr(not(feature = "cli"), allow(dead_code))]
pub(crate) struct PolicyExpansion {
    pub(crate) bootstrap_type: TokenStream,
    pub(crate) serve_enabled: bool,
    pub(crate) serve_default: bool,
    pub(crate) serve_attributes: TokenStream,
    pub(crate) serve_definition: Ident,
}

struct EffectiveArgument<'a> {
    policy: Option<&'a ArgumentPolicy>,
    key: Option<&'a Ident>,
    enabled: bool,
    name: LitStr,
    short: Option<LitChar>,
    help: LitStr,
    value_name: LitStr,
}

struct EffectiveServe<'a> {
    policy: Option<&'a ServePolicy>,
    enabled: bool,
    default_command: bool,
    name: LitStr,
    help: LitStr,
    key: Option<&'a Ident>,
}

/// Resolves omission-preserving policy into generated parser and bootstrap Rust.
pub(crate) fn expand(
    policy: &CliPolicy,
    serve_phase: Option<&Declared<PhaseInput>>,
    visibility: &Visibility,
    host_ident: &Ident,
    paths: &Paths,
) -> syn::Result<PolicyExpansion> {
    let has_serve_phase = serve_phase.is_some();

    if !has_serve_phase
        && policy
            .serve
            .as_ref()
            .is_some_and(|serve| serve.value.enabled)
    {
        return Err(syn::Error::new(
            policy
                .serve
                .as_ref()
                .expect("enabled explicit serve policy exists")
                .key
                .span(),
            "the reserved `serve` CLI slot cannot be enabled without a declared serve phase",
        ));
    }

    let config = effective_argument(
        policy
            .config
            .as_ref()
            .map(|value| (&value.key, &value.value)),
        "config",
        Some('c'),
        "Configuration file or directory.",
        "PATH",
    );
    let profile = effective_argument(
        policy
            .profile
            .as_ref()
            .map(|value| (&value.key, &value.value)),
        "profile",
        Some('p'),
        "Ordered configuration profile; may be repeated.",
        "PROFILE",
    );
    let log = effective_argument(
        policy.log.as_ref().map(|value| (&value.key, &value.value)),
        "log",
        None,
        "EnvFilter-compatible tracing directive.",
        "FILTER",
    );
    let log_format = effective_argument(
        policy
            .log_format
            .as_ref()
            .map(|value| (&value.key, &value.value.argument)),
        "log-format",
        None,
        "Tracing output formatter.",
        "FORMAT",
    );
    let color = effective_argument(
        policy
            .color
            .as_ref()
            .map(|value| (&value.key, &value.value.argument)),
        "color",
        None,
        "ANSI color behavior.",
        "WHEN",
    );
    let serve = effective_serve(policy.serve.as_ref(), serve_phase);
    let bootstrap_ident = format_ident!("{}BootstrapArgs", host_ident, span = host_ident.span());
    let bootstrap_options = paths.core("BootstrapOptions");
    let log_format_type = paths.core("LogFormat");
    let color_type = paths.core("ColorChoice");
    let config_typed = has_typed_scalar_default(&config);
    let log_typed = has_typed_scalar_default(&log);
    let log_format_typed = has_typed_scalar_default(&log_format);
    let color_typed = has_typed_scalar_default(&color);
    let config_field = argument_field(
        &config,
        scalar_type(quote!(::std::path::PathBuf), config_typed),
        field_definition(&config, "config", host_ident, false),
        false,
    );
    let profile_field = argument_field(
        &profile,
        quote!(Vec<::std::string::String>),
        field_definition(&profile, "profiles", host_ident, true),
        false,
    );
    let log_field = argument_field(
        &log,
        scalar_type(quote!(::std::string::String), log_typed),
        field_definition(&log, "log", host_ident, false),
        false,
    );
    let log_format_field = argument_field(
        &log_format,
        scalar_type(quote!(#log_format_type), log_format_typed),
        field_definition(&log_format, "log_format", host_ident, false),
        true,
    );
    let color_field = argument_field(
        &color,
        scalar_type(quote!(#color_type), color_typed),
        field_definition(&color, "color", host_ident, false),
        true,
    );
    let config_value = scalar_value(&config, quote!(self.config), config_typed);
    let profile_value = enabled_value(
        profile.enabled,
        quote!(self.profiles),
        quote!(::std::vec::Vec::new()),
    );
    let log_value = scalar_value(&log, quote!(self.log), log_typed);
    let log_format_value = scalar_value(&log_format, quote!(self.log_format), log_format_typed);
    let color_value = scalar_value(&color, quote!(self.color), color_typed);
    let config_source = value_source(config.enabled, "config");
    let profile_source = value_source(profile.enabled, "profiles");
    let log_source = value_source(log.enabled, "log");
    let log_format_source = value_source(log_format.enabled, "log_format");
    let color_source = value_source(color.enabled, "color");
    let serve_attributes = serve_attributes(&serve);

    Ok(PolicyExpansion {
        bootstrap_type: quote! {
            /// Generated application-specific framework bootstrap arguments.
            #[derive(::clap::Args)]
            #visibility struct #bootstrap_ident {
                #config_field
                #profile_field
                #log_field
                #log_format_field
                #color_field
            }

            impl #bootstrap_ident {
                fn __sources(
                    matches: &::clap::ArgMatches,
                ) -> [::core::option::Option<::clap::parser::ValueSource>; 5] {
                    [
                        #config_source,
                        #profile_source,
                        #log_source,
                        #log_format_source,
                        #color_source,
                    ]
                }

                fn __into_options(
                    self,
                    sources: [::core::option::Option<::clap::parser::ValueSource>; 5],
                ) -> #bootstrap_options {
                    #bootstrap_options::from_parts(
                        #config_value,
                        #profile_value,
                        #log_value,
                        #log_format_value,
                        #color_value,
                        sources,
                    )
                }
            }
        },
        serve_enabled: serve.enabled,
        serve_default: serve.default_command,
        serve_attributes,
        serve_definition: Ident::new(
            "Serve",
            Span::call_site().located_at(
                serve
                    .key
                    .map(Ident::span)
                    .unwrap_or_else(|| host_ident.span()),
            ),
        ),
    })
}

fn effective_argument<'a>(
    policy: Option<(&'a Ident, &'a ArgumentPolicy)>,
    default_name: &str,
    default_short: Option<char>,
    default_help: &str,
    default_value_name: &str,
) -> EffectiveArgument<'a> {
    let key = policy.map(|(key, _)| key);
    let policy = policy.map(|(_, policy)| policy);
    let enabled = policy.is_none_or(|policy| policy.enabled);
    let name = policy
        .and_then(|policy| policy.name.clone())
        .unwrap_or_else(|| LitStr::new(default_name, Span::call_site()));
    let short = match policy.and_then(|policy| policy.short.as_ref()) {
        Some(short) => short.clone(),
        None => default_short.map(|short| LitChar::new(short, Span::call_site())),
    };
    let help = policy
        .and_then(|policy| policy.help.clone())
        .unwrap_or_else(|| LitStr::new(default_help, Span::call_site()));
    let value_name = policy
        .and_then(|policy| policy.value_name.clone())
        .unwrap_or_else(|| LitStr::new(default_value_name, Span::call_site()));

    EffectiveArgument {
        policy,
        key,
        enabled,
        name,
        short,
        help,
        value_name,
    }
}

fn effective_serve<'a>(
    policy: Option<&'a Declared<ServePolicy>>,
    serve_phase: Option<&'a Declared<PhaseInput>>,
) -> EffectiveServe<'a> {
    let has_serve_phase = serve_phase.is_some();
    let key = policy
        .map(|policy| &policy.key)
        .or_else(|| serve_phase.map(|phase| &phase.key));
    let policy = policy.map(|policy| &policy.value);
    let enabled = has_serve_phase && policy.is_none_or(|policy| policy.enabled);
    let default_command = enabled
        && policy
            .and_then(|policy| policy.default_command)
            .unwrap_or(true);
    let name = policy
        .and_then(|policy| policy.name.clone())
        .unwrap_or_else(|| LitStr::new("serve", Span::call_site()));
    let help = policy
        .and_then(|policy| policy.help.clone())
        .unwrap_or_else(|| LitStr::new("Build and serve the application.", Span::call_site()));

    EffectiveServe {
        policy,
        enabled,
        default_command,
        name,
        help,
        key,
    }
}

fn field_definition(
    argument: &EffectiveArgument<'_>,
    name: &str,
    host_ident: &Ident,
    alias: bool,
) -> Ident {
    let Some(key) = argument.key else {
        return format_ident!("{name}", span = host_ident.span());
    };

    if alias {
        return Ident::new(name, Span::call_site().located_at(key.span()));
    }

    key.clone()
}

fn argument_field(
    argument: &EffectiveArgument<'_>,
    ty: TokenStream,
    field: Ident,
    value_enum: bool,
) -> TokenStream {
    if !argument.enabled {
        return TokenStream::new();
    }

    let name = &argument.name;
    let help = &argument.help;
    let value_name = &argument.value_name;
    let hidden = argument.policy.is_some_and(|policy| policy.hidden);
    let short = argument.short.as_ref().map(|short| quote!(short = #short,));
    let aliases = argument
        .policy
        .map(|policy| policy.aliases.as_slice())
        .unwrap_or_default();
    let visible_aliases = argument
        .policy
        .map(|policy| policy.visible_aliases.as_slice())
        .unwrap_or_default();
    let aliases = (!aliases.is_empty()).then(|| quote!(aliases = [#(#aliases),*],));
    let visible_aliases =
        (!visible_aliases.is_empty()).then(|| quote!(visible_aliases = [#(#visible_aliases),*],));
    let value_enum = value_enum.then(|| quote!(value_enum,));
    let default = match argument
        .policy
        .and_then(|policy| policy.clap_default.as_ref())
    {
        None => None,
        Some(ArgumentDefault::DefaultValue(default)) => Some(quote!(default_value = #default,)),
        Some(ArgumentDefault::DefaultValues(defaults)) => {
            Some(quote!(default_values = [#(#defaults),*],))
        }
        Some(ArgumentDefault::DefaultValueT(default)) => Some(quote!(default_value_t = #default,)),
        Some(ArgumentDefault::DefaultValuesT(defaults)) => {
            Some(quote!(default_values_t = #defaults,))
        }
    };

    quote! {
        #[arg(
            long = #name,
            #short
            #aliases
            #visible_aliases
            global = true,
            value_name = #value_name,
            help = #help,
            hide = #hidden,
            #value_enum
            #default
        )]
        pub #field: #ty,
    }
}

fn serve_attributes(serve: &EffectiveServe<'_>) -> TokenStream {
    let name = &serve.name;
    let help = &serve.help;
    let hidden = serve.policy.is_some_and(|policy| policy.hidden);
    let aliases = serve
        .policy
        .map(|policy| policy.aliases.as_slice())
        .unwrap_or_default();
    let visible_aliases = serve
        .policy
        .map(|policy| policy.visible_aliases.as_slice())
        .unwrap_or_default();
    let aliases = (!aliases.is_empty()).then(|| quote!(aliases = [#(#aliases),*],));
    let visible_aliases =
        (!visible_aliases.is_empty()).then(|| quote!(visible_aliases = [#(#visible_aliases),*],));

    quote! {
        #[command(
            name = #name,
            #aliases
            #visible_aliases
            hide = #hidden,
            about = #help,
        )]
    }
}

fn enabled_value(
    enabled: bool,
    enabled_value: TokenStream,
    disabled_value: TokenStream,
) -> TokenStream {
    if enabled {
        return enabled_value;
    }

    disabled_value
}

fn has_typed_scalar_default(argument: &EffectiveArgument<'_>) -> bool {
    argument
        .policy
        .and_then(|policy| policy.clap_default.as_ref())
        .is_some_and(ArgumentDefault::is_typed_scalar)
}

fn scalar_type(ty: TokenStream, typed: bool) -> TokenStream {
    if typed {
        return ty;
    }

    quote!(Option<#ty>)
}

fn scalar_value(argument: &EffectiveArgument<'_>, value: TokenStream, typed: bool) -> TokenStream {
    if !argument.enabled {
        return quote!(None);
    }

    if typed {
        return quote!(Some(#value));
    }

    value
}

fn value_source(enabled: bool, id: &str) -> TokenStream {
    if enabled {
        let id = LitStr::new(id, Span::call_site());

        return quote!(matches.value_source(#id));
    }

    quote!(::core::option::Option::None)
}
