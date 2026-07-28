use std::collections::HashSet;

use proc_macro2::TokenStream;
use quote::ToTokens;
use syn::ext::IdentExt as _;
use syn::parse::ParseStream;
use syn::{Expr, Ident, LitBool, LitChar, LitStr, Token, braced, bracketed};

use super::model::Declared;

mod expand;

#[cfg(feature = "cli")]
pub(super) use expand::PolicyExpansion;
pub(super) use expand::expand;

/// Parsed policy for framework-reserved command-line slots.
#[derive(Default)]
pub(super) struct CliPolicy {
    pub(super) config: Option<Declared<ArgumentPolicy>>,
    pub(super) profile: Option<Declared<ArgumentPolicy>>,
    pub(super) log: Option<Declared<ArgumentPolicy>>,
    pub(super) log_format: Option<Declared<ChoiceArgumentPolicy>>,
    pub(super) color: Option<Declared<ChoiceArgumentPolicy>>,
    pub(super) serve: Option<Declared<ServePolicy>>,
}

/// Customization shared by framework bootstrap arguments.
pub(super) struct ArgumentPolicy {
    pub(super) enabled: bool,
    pub(super) name: Option<LitStr>,
    pub(super) short: Option<Option<LitChar>>,
    pub(super) aliases: Vec<LitStr>,
    pub(super) visible_aliases: Vec<LitStr>,
    pub(super) hidden: bool,
    pub(super) help: Option<LitStr>,
    pub(super) value_name: Option<LitStr>,
    pub(super) clap_default: Option<ArgumentDefault>,
}

/// One exact Clap default attribute declared for a reserved argument.
pub(super) enum ArgumentDefault {
    DefaultValue(LitStr),
    DefaultValues(Vec<LitStr>),
    DefaultValueT(Expr),
    DefaultValuesT(Expr),
}

impl ArgumentDefault {
    fn name(&self) -> &'static str {
        match self {
            Self::DefaultValue(_) => "default_value",
            Self::DefaultValues(_) => "default_values",
            Self::DefaultValueT(_) => "default_value_t",
            Self::DefaultValuesT(_) => "default_values_t",
        }
    }

    pub(super) fn is_typed_scalar(&self) -> bool {
        matches!(self, Self::DefaultValueT(_))
    }
}

impl ToTokens for ArgumentDefault {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            Self::DefaultValue(value) => value.to_tokens(tokens),
            Self::DefaultValues(values) => {
                for value in values {
                    value.to_tokens(tokens);
                }
            }
            Self::DefaultValueT(value) | Self::DefaultValuesT(value) => value.to_tokens(tokens),
        }
    }
}

/// Bootstrap argument customization with a validated finite application default.
#[derive(Default)]
pub(super) struct ChoiceArgumentPolicy {
    pub(super) argument: ArgumentPolicy,
}

/// Customization of the generated framework serve command.
pub(super) struct ServePolicy {
    pub(super) enabled: bool,
    pub(super) name: Option<LitStr>,
    pub(super) aliases: Vec<LitStr>,
    pub(super) visible_aliases: Vec<LitStr>,
    pub(super) hidden: bool,
    pub(super) help: Option<LitStr>,
    pub(super) default_command: Option<bool>,
}

impl Default for ArgumentPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            name: None,
            short: None,
            aliases: Vec::new(),
            visible_aliases: Vec::new(),
            hidden: false,
            help: None,
            value_name: None,
            clap_default: None,
        }
    }
}

impl Default for ServePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            name: None,
            aliases: Vec::new(),
            visible_aliases: Vec::new(),
            hidden: false,
            help: None,
            default_command: None,
        }
    }
}

pub(super) fn parse(input: ParseStream) -> syn::Result<CliPolicy> {
    let content;
    let mut policy = CliPolicy::default();
    let mut keys = HashSet::new();

    braced!(content in input);

    while !content.is_empty() {
        let key = content.call(Ident::parse_any)?;
        let name = key.unraw().to_string();

        if !keys.insert(name.clone()) {
            return Err(syn::Error::new(
                key.span(),
                format!("duplicate reserved CLI slot `{name}`"),
            ));
        }

        content.parse::<Token![:]>()?;

        match name.as_str() {
            "config" | "profile" | "log" => {
                let slot = parse_argument(&content)?;

                validate_default_shape(&name, &slot)?;

                match name.as_str() {
                    "config" => policy.config = Some(Declared { key, value: slot }),
                    "profile" => policy.profile = Some(Declared { key, value: slot }),
                    "log" => policy.log = Some(Declared { key, value: slot }),
                    _ => unreachable!(),
                }
            }
            "log_format" => {
                policy.log_format = Some(Declared {
                    key,
                    value: parse_choice_argument(&content, &["full", "compact", "pretty", "json"])?,
                });
            }
            "color" => {
                policy.color = Some(Declared {
                    key,
                    value: parse_choice_argument(&content, &["auto", "always", "never"])?,
                });
            }
            "serve" => {
                let value = parse_serve(&content)?;

                if !value.enabled && value.default_command == Some(true) {
                    return Err(syn::Error::new(
                        key.span(),
                        "disabled `serve` cannot be the default command",
                    ));
                }

                policy.serve = Some(Declared { key, value });
            }
            other => {
                return Err(syn::Error::new(
                    key.span(),
                    format!(
                        "unknown reserved CLI slot `{other}`; expected `config`, `profile`, `log`, `log_format`, `color`, or `serve`"
                    ),
                ));
            }
        }

        parse_separator(&content)?;
    }

    Ok(policy)
}

fn parse_argument(input: ParseStream) -> syn::Result<ArgumentPolicy> {
    if input.peek(LitBool) {
        let enabled = input.parse::<LitBool>()?.value;

        return Ok(ArgumentPolicy {
            enabled,
            ..ArgumentPolicy::default()
        });
    }

    let content;
    let mut policy = ArgumentPolicy::default();
    let mut keys = HashSet::new();

    braced!(content in input);

    while !content.is_empty() {
        let key = content.call(Ident::parse_any)?;
        let name = key.unraw().to_string();

        duplicate_setting(&mut keys, &key, &name)?;
        content.parse::<Token![:]>()?;

        match name.as_str() {
            "enabled" => policy.enabled = content.parse::<LitBool>()?.value,
            "name" => policy.name = Some(parse_nonempty_string(&content, "option name")?),
            "short" => policy.short = Some(parse_short(&content)?),
            "aliases" => policy.aliases = parse_strings(&content, "option alias")?,
            "visible_aliases" => {
                policy.visible_aliases = parse_strings(&content, "visible option alias")?;
            }
            "hidden" => policy.hidden = content.parse::<LitBool>()?.value,
            "help" => policy.help = Some(content.parse()?),
            "value_name" => {
                policy.value_name = Some(parse_nonempty_string(&content, "value name")?);
            }
            "default_value" => set_default(
                &mut policy.clap_default,
                ArgumentDefault::DefaultValue(content.parse()?),
            )?,
            "default_values" => set_default(
                &mut policy.clap_default,
                ArgumentDefault::DefaultValues(parse_strings(&content, "application default")?),
            )?,
            "default_value_t" => set_default(
                &mut policy.clap_default,
                ArgumentDefault::DefaultValueT(content.parse()?),
            )?,
            "default_values_t" => set_default(
                &mut policy.clap_default,
                ArgumentDefault::DefaultValuesT(content.parse()?),
            )?,
            other => {
                return Err(syn::Error::new(
                    key.span(),
                    format!(
                        "unknown reserved argument setting `{other}`; expected `enabled`, `name`, `short`, `aliases`, `visible_aliases`, `hidden`, `help`, `value_name`, `default_value`, `default_values`, `default_value_t`, or `default_values_t`"
                    ),
                ));
            }
        }

        parse_separator(&content)?;
    }

    validate_argument(&policy)?;

    Ok(policy)
}

fn parse_choice_argument(
    input: ParseStream,
    choices: &[&str],
) -> syn::Result<ChoiceArgumentPolicy> {
    let argument = parse_argument(input)?;

    validate_default_shape("choice", &argument)?;

    if let Some(ArgumentDefault::DefaultValue(default)) = &argument.clap_default {
        validate_choice(default, choices)?;
    }

    Ok(ChoiceArgumentPolicy { argument })
}

fn parse_serve(input: ParseStream) -> syn::Result<ServePolicy> {
    if input.peek(LitBool) {
        let enabled = input.parse::<LitBool>()?.value;

        return Ok(ServePolicy {
            enabled,
            ..ServePolicy::default()
        });
    }

    let content;
    let mut policy = ServePolicy::default();
    let mut keys = HashSet::new();

    braced!(content in input);

    while !content.is_empty() {
        let key = content.call(Ident::parse_any)?;
        let name = key.unraw().to_string();

        duplicate_setting(&mut keys, &key, &name)?;
        content.parse::<Token![:]>()?;

        match name.as_str() {
            "enabled" => policy.enabled = content.parse::<LitBool>()?.value,
            "name" => policy.name = Some(parse_nonempty_string(&content, "command name")?),
            "aliases" => policy.aliases = parse_strings(&content, "command alias")?,
            "visible_aliases" => {
                policy.visible_aliases = parse_strings(&content, "visible command alias")?;
            }
            "hidden" => policy.hidden = content.parse::<LitBool>()?.value,
            "help" => policy.help = Some(content.parse()?),
            "default_command" => {
                policy.default_command = Some(content.parse::<LitBool>()?.value);
            }
            other => {
                return Err(syn::Error::new(
                    key.span(),
                    format!(
                        "unknown reserved serve setting `{other}`; expected `enabled`, `name`, `aliases`, `visible_aliases`, `hidden`, `help`, or `default_command`"
                    ),
                ));
            }
        }

        parse_separator(&content)?;
    }

    validate_names(
        policy.name.as_ref(),
        &policy.aliases,
        &policy.visible_aliases,
    )?;

    Ok(policy)
}

fn validate_argument(policy: &ArgumentPolicy) -> syn::Result<()> {
    validate_names(
        policy.name.as_ref(),
        &policy.aliases,
        &policy.visible_aliases,
    )?;

    if policy
        .short
        .as_ref()
        .and_then(|short| short.as_ref())
        .is_some_and(|short| short.value() == '-')
    {
        return Err(syn::Error::new_spanned(
            policy.short.as_ref().and_then(|short| short.as_ref()),
            "short option cannot be `-`",
        ));
    }

    if !policy.enabled
        && let Some(default) = &policy.clap_default
    {
        return Err(syn::Error::new_spanned(
            default,
            "disabled reserved CLI arguments cannot declare a default",
        ));
    }

    Ok(())
}

fn validate_default_shape(slot: &str, policy: &ArgumentPolicy) -> syn::Result<()> {
    let Some(default) = &policy.clap_default else {
        return Ok(());
    };
    let plural = matches!(
        default,
        ArgumentDefault::DefaultValues(_) | ArgumentDefault::DefaultValuesT(_)
    );
    let expects_plural = slot == "profile";

    if plural == expects_plural {
        return Ok(());
    }

    let expected = if expects_plural {
        "`default_values` or `default_values_t`"
    } else {
        "`default_value` or `default_value_t`"
    };

    Err(syn::Error::new_spanned(
        default,
        format!(
            "reserved `{slot}` does not accept `{}`; use {expected}",
            default.name()
        ),
    ))
}

fn set_default(target: &mut Option<ArgumentDefault>, value: ArgumentDefault) -> syn::Result<()> {
    if let Some(existing) = target {
        return Err(syn::Error::new_spanned(
            &value,
            format!(
                "reserved CLI arguments cannot combine `{}` with `{}`",
                existing.name(),
                value.name()
            ),
        ));
    }

    *target = Some(value);

    Ok(())
}

fn validate_names(
    name: Option<&LitStr>,
    aliases: &[LitStr],
    visible_aliases: &[LitStr],
) -> syn::Result<()> {
    for literal in name
        .into_iter()
        .chain(aliases.iter())
        .chain(visible_aliases.iter())
    {
        let value = literal.value();

        if value.starts_with('-') {
            return Err(syn::Error::new_spanned(
                literal,
                "reserved CLI names omit leading dashes",
            ));
        }
    }

    Ok(())
}

fn validate_choice(literal: &LitStr, choices: &[&str]) -> syn::Result<()> {
    let value = literal.value();

    if !choices.contains(&value.as_str()) {
        return Err(syn::Error::new_spanned(
            literal,
            format!(
                "unsupported application default `{value}`; expected one of: {}",
                choices.join(", ")
            ),
        ));
    }

    Ok(())
}

fn parse_short(input: ParseStream) -> syn::Result<Option<LitChar>> {
    if input.peek(LitBool) {
        let enabled = input.parse::<LitBool>()?;

        if enabled.value {
            return Err(syn::Error::new_spanned(
                enabled,
                "use a character literal for `short`, or `false` to disable it",
            ));
        }

        return Ok(None);
    }

    Ok(Some(input.parse()?))
}

fn parse_strings(input: ParseStream, description: &str) -> syn::Result<Vec<LitStr>> {
    let content;

    bracketed!(content in input);

    let values: Vec<LitStr> = content
        .parse_terminated(|input| input.parse(), Token![,])?
        .into_iter()
        .collect::<Vec<_>>();

    for value in &values {
        if value.value().is_empty() {
            return Err(syn::Error::new_spanned(
                value,
                format!("{description} cannot be empty"),
            ));
        }
    }

    Ok(values)
}

fn parse_nonempty_string(input: ParseStream, description: &str) -> syn::Result<LitStr> {
    let value = input.parse::<LitStr>()?;

    if value.value().is_empty() {
        return Err(syn::Error::new_spanned(
            &value,
            format!("{description} cannot be empty"),
        ));
    }

    Ok(value)
}

fn duplicate_setting(keys: &mut HashSet<String>, key: &Ident, name: &str) -> syn::Result<()> {
    if !keys.insert(name.to_owned()) {
        return Err(syn::Error::new(
            key.span(),
            format!("duplicate reserved CLI setting `{name}`"),
        ));
    }

    Ok(())
}

fn parse_separator(input: ParseStream) -> syn::Result<()> {
    if input.peek(Token![,]) {
        input.parse::<Token![,]>()?;
    } else if !input.is_empty() {
        return Err(input.error("expected `,` between reserved CLI declarations"));
    }

    Ok(())
}
