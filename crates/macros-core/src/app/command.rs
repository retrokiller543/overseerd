use std::collections::HashSet;

#[cfg(feature = "cli")]
use proc_macro2::TokenStream;
#[cfg(feature = "cli")]
use quote::quote;
use quote::{ToTokens as _, format_ident};
use syn::ext::IdentExt as _;
use syn::parse::{Parse as _, ParseStream};
use syn::{Attribute, Ident, Token, Type, braced};
#[cfg(feature = "cli")]
use syn::{Path, Visibility};

use super::model::{CommandEntry, CommandEntryKind, GlobalArgsEntry};

/// Generated command variants, delegation arms, and nested enum definitions.
#[cfg(feature = "cli")]
pub(super) struct CommandExpansion {
    pub(super) variants: TokenStream,
    pub(super) phase_arms: TokenStream,
    pub(super) run_arms: TokenStream,
    pub(super) nested_types: TokenStream,
}

/// Inputs shared by recursive command-tree expansion.
#[cfg(feature = "cli")]
pub(super) struct ExpansionInput<'a> {
    pub(super) visibility: &'a Visibility,
    pub(super) host_ident: &'a Ident,
    pub(super) host: &'a proc_macro2::TokenStream,
    pub(super) entries: &'a [CommandEntry],
    pub(super) cli_command: &'a Path,
    pub(super) command_context: &'a Path,
    pub(super) command_error: &'a Path,
    pub(super) command_phase: &'a Path,
}

pub(super) fn parse_args(input: ParseStream) -> syn::Result<Vec<GlobalArgsEntry>> {
    let content;
    let mut aliases = HashSet::new();
    let mut types = HashSet::new();
    let mut entries = Vec::new();

    braced!(content in input);

    while !content.is_empty() {
        let attributes = content.call(Attribute::parse_outer)?;
        let alias = content.call(Ident::parse_any)?;

        validate_global_args_attributes(&attributes)?;

        content.parse::<Token![:]>()?;

        let ty: Type = content.parse()?;
        let alias_name = alias.unraw().to_string();
        let type_name = ty.to_token_stream().to_string();

        if matches!(alias_name.as_str(), "bootstrap" | "command") {
            return Err(syn::Error::new(
                alias.span(),
                format!("global argument alias `{alias_name}` is reserved by the generated CLI"),
            ));
        }

        if aliases.contains(&alias_name) {
            return Err(syn::Error::new(
                alias.span(),
                format!("duplicate global argument alias `{alias_name}`"),
            ));
        }

        aliases.insert(alias_name);

        if !types.insert(type_name) {
            return Err(syn::Error::new_spanned(
                &ty,
                "a global argument type can only be registered once",
            ));
        }

        entries.push(GlobalArgsEntry {
            attributes,
            alias,
            ty,
        });

        parse_separator(&content)?;
    }

    Ok(entries)
}

pub(super) fn parse_commands(input: ParseStream) -> syn::Result<Vec<CommandEntry>> {
    let content;

    braced!(content in input);

    let commands = parse_command_entries(&content, true)?;

    if commands.is_empty() {
        return Err(content.error("`commands` cannot be empty"));
    }

    Ok(commands)
}

#[cfg(feature = "cli")]
pub(super) fn expand(input: ExpansionInput<'_>) -> syn::Result<CommandExpansion> {
    let mut nested_names = HashSet::new();
    let mut path = Vec::new();

    expand_entries(&input, input.entries, &mut path, &mut nested_names)
}

#[cfg(feature = "cli")]
fn expand_entries<'a>(
    input: &ExpansionInput<'_>,
    entries: &'a [CommandEntry],
    path: &mut Vec<&'a Ident>,
    nested_names: &mut HashSet<String>,
) -> syn::Result<CommandExpansion> {
    let mut variants = TokenStream::new();
    let mut phase_arms = TokenStream::new();
    let mut run_arms = TokenStream::new();
    let mut nested_types = TokenStream::new();

    for entry in entries {
        let attributes = &entry.attributes;
        let variant = variant_ident(&entry.name);
        let command_path = path_string(path, &entry.name);
        let command_name = normalize_name(&entry.name);

        match &entry.kind {
            CommandEntryKind::Leaf(ty) => {
                let cli_command = input.cli_command;
                let host = input.host;
                let command_error = input.command_error;

                variants.extend(quote! {
                    #(#attributes)*
                    #[command(name = #command_name)]
                    #variant(#ty),
                });
                phase_arms.extend(quote! {
                    Self::#variant(command) => <#ty as #cli_command<#host>>::phase(command),
                });
                run_arms.extend(quote! {
                    Self::#variant(command) => {
                        <#ty as #cli_command<#host>>::run(command, context)
                            .await
                            .map_err(|source| #command_error::new(#command_path, source))?;

                        Ok(())
                    }
                });
            }
            CommandEntryKind::Namespace(children) => {
                path.push(&entry.name);

                let nested_ident = nested_command_ident(input.host_ident, path);
                let nested_name = nested_ident.to_string();

                if nested_names.contains(&nested_name) {
                    return Err(syn::Error::new(
                        entry.name.span(),
                        format!("command namespace generates duplicate type `{nested_name}`"),
                    ));
                }

                nested_names.insert(nested_name);

                let nested = expand_entries(input, children, path, nested_names)?;
                let visibility = input.visibility;
                let cli_command = input.cli_command;
                let command_context = input.command_context;
                let command_error = input.command_error;
                let command_phase = input.command_phase;
                let host = input.host;
                let clap: Path = syn::parse_quote!(::clap);
                let nested_variants = nested.variants;
                let nested_phase_arms = nested.phase_arms;
                let nested_run_arms = nested.run_arms;

                variants.extend(quote! {
                    #(#attributes)*
                    #[command(name = #command_name)]
                    #variant {
                        #[command(subcommand)]
                        command: #nested_ident,
                    },
                });
                phase_arms.extend(quote! {
                    Self::#variant { command } => <#nested_ident as #cli_command<#host>>::phase(command),
                });
                run_arms.extend(quote! {
                    Self::#variant { command } => {
                        <#nested_ident as #cli_command<#host>>::run(command, context).await?;

                        Ok(())
                    }
                });
                nested_types.extend(nested.nested_types);
                nested_types.extend(quote! {
                    /// Generated nested application commands.
                    #[derive(#clap::Subcommand)]
                    #visibility enum #nested_ident {
                        #nested_variants
                    }

                    impl #cli_command<#host> for #nested_ident {
                        type Error = #command_error;

                        fn phase(&self) -> #command_phase {
                            match self {
                                #nested_phase_arms
                            }
                        }

                        async fn run(
                            &self,
                            context: #command_context<#host>,
                        ) -> ::core::result::Result<(), Self::Error> {
                            match self {
                                #nested_run_arms
                            }
                        }
                    }
                });

                path.pop();
            }
        }
    }

    Ok(CommandExpansion {
        variants,
        phase_arms,
        run_arms,
        nested_types,
    })
}

fn parse_command_entries(input: ParseStream, root: bool) -> syn::Result<Vec<CommandEntry>> {
    let mut names = HashSet::new();
    let mut variants = HashSet::new();
    let mut entries = Vec::new();

    while !input.is_empty() {
        let attributes = input.call(Attribute::parse_outer)?;
        let name = input.call(Ident::parse_any)?;
        let normalized = normalize_name(&name);

        validate_attributes(&attributes)?;

        if names.contains(&normalized) {
            return Err(syn::Error::new(
                name.span(),
                format!("duplicate command name `{normalized}`"),
            ));
        }

        let variant = variant_ident(&name).to_string();

        if variants.contains(&variant) {
            return Err(syn::Error::new(
                name.span(),
                format!("command name generates duplicate Rust variant `{variant}`"),
            ));
        }

        variants.insert(variant);

        if normalized == "help" {
            return Err(syn::Error::new(
                name.span(),
                "command name `help` is reserved by Clap",
            ));
        }

        if root && normalized == "serve" {
            return Err(syn::Error::new(
                name.span(),
                "command name `serve` is reserved by the generated application CLI",
            ));
        }

        if normalized.starts_with("--overseerd") || normalized.starts_with("__overseerd") {
            return Err(syn::Error::new(
                name.span(),
                "command names beginning with `__overseerd` are reserved for framework tooling",
            ));
        }

        input.parse::<Token![:]>()?;

        let kind = if input.peek(syn::token::Brace) {
            let content;

            braced!(content in input);

            let nested = parse_command_entries(&content, false)?;

            if nested.is_empty() {
                return Err(syn::Error::new(
                    name.span(),
                    format!("command namespace `{normalized}` cannot be empty"),
                ));
            }

            CommandEntryKind::Namespace(nested)
        } else {
            CommandEntryKind::Leaf(input.parse()?)
        };

        names.insert(normalized);

        entries.push(CommandEntry {
            attributes,
            name,
            kind,
        });

        parse_separator(input)?;
    }

    Ok(entries)
}

fn validate_global_args_attributes(attributes: &[Attribute]) -> syn::Result<()> {
    for attribute in attributes {
        if !attribute.path().is_ident("doc") {
            return Err(syn::Error::new_spanned(
                attribute,
                "only documentation attributes are supported on flattened argument groups",
            ));
        }
    }

    Ok(())
}

fn validate_attributes(attributes: &[Attribute]) -> syn::Result<()> {
    for attribute in attributes {
        if attribute.path().is_ident("doc") {
            continue;
        }

        if !attribute.path().is_ident("command") {
            return Err(syn::Error::new_spanned(
                attribute,
                "only documentation and `command` attributes are supported on CLI declarations",
            ));
        }

        attribute.parse_nested_meta(validate_command_attribute)?;
    }

    Ok(())
}

fn validate_command_attribute(meta: syn::meta::ParseNestedMeta<'_>) -> syn::Result<()> {
    let name = meta
        .path
        .get_ident()
        .ok_or_else(|| meta.error("Clap command setting must be a single identifier"))?
        .to_string();

    if matches!(
        name.as_str(),
        "name"
            | "ignore_errors"
            | "rename_all"
            | "rename_all_env"
            | "flatten"
            | "subcommand"
            | "external_subcommand"
            | "skip"
            | "allow_external_subcommands"
            | "subcommand_required"
    ) {
        return Err(meta.error(format!(
            "Clap command setting `{name}` would change generated command dispatch"
        )));
    }

    if !matches!(
        name.as_str(),
        "no_binary_name"
            | "args_override_self"
            | "dont_delimit_trailing_values"
            | "color"
            | "styles"
            | "term_width"
            | "max_term_width"
            | "disable_version_flag"
            | "propagate_version"
            | "next_line_help"
            | "disable_help_flag"
            | "disable_help_subcommand"
            | "disable_colored_help"
            | "help_expected"
            | "dont_collapse_args_in_usage"
            | "hide_possible_values"
            | "infer_long_args"
            | "infer_subcommands"
            | "bin_name"
            | "display_name"
            | "author"
            | "about"
            | "long_about"
            | "before_help"
            | "before_long_help"
            | "after_help"
            | "after_long_help"
            | "version"
            | "long_version"
            | "override_usage"
            | "override_help"
            | "help_template"
            | "flatten_help"
            | "alias"
            | "aliases"
            | "visible_alias"
            | "visible_aliases"
            | "short_flag"
            | "long_flag"
            | "short_flag_alias"
            | "long_flag_alias"
            | "short_flag_aliases"
            | "long_flag_aliases"
            | "visible_short_flag_alias"
            | "visible_long_flag_alias"
            | "visible_short_flag_aliases"
            | "visible_long_flag_aliases"
            | "hide"
            | "display_order"
            | "next_display_order"
            | "next_help_heading"
            | "arg_required_else_help"
            | "allow_hyphen_values"
            | "allow_negative_numbers"
            | "trailing_var_arg"
            | "allow_missing_positional"
            | "args_conflicts_with_subcommands"
            | "subcommand_precedence_over_arg"
            | "subcommand_negates_reqs"
            | "subcommand_value_name"
            | "subcommand_help_heading"
            | "verbatim_doc_comment"
    ) {
        return Err(meta.error(format!(
            "Clap command setting `{name}` is not supported by the generated command DSL"
        )));
    }

    if meta.input.peek(Token![=]) {
        let value = meta.value()?;
        let _: syn::Expr = value.parse()?;
    } else if meta.input.peek(syn::token::Paren) {
        let content;

        syn::parenthesized!(content in meta.input);

        let _ = content.parse_terminated(syn::Expr::parse, Token![,])?;
    } else if !matches!(
        name.as_str(),
        "about" | "long_about" | "author" | "version" | "verbatim_doc_comment"
    ) {
        return Err(meta.error(format!("Clap command setting `{name}` requires a value")));
    }

    Ok(())
}

fn parse_separator(input: ParseStream) -> syn::Result<()> {
    if input.peek(Token![,]) {
        input.parse::<Token![,]>()?;
    } else if !input.is_empty() {
        return Err(input.error("expected `,` between CLI declarations"));
    }

    Ok(())
}

fn normalize_name(name: &Ident) -> String {
    name.unraw().to_string().to_lowercase().replace('_', "-")
}

fn variant_ident(name: &Ident) -> Ident {
    format_ident!(
        "{}",
        pascal_case(&name.unraw().to_string()),
        span = name.span()
    )
}

#[cfg(feature = "cli")]
fn nested_command_ident(host: &Ident, path: &[&Ident]) -> Ident {
    let suffix = path
        .iter()
        .map(|segment| pascal_case(&segment.unraw().to_string()))
        .collect::<String>();

    format_ident!("{host}{suffix}Command")
}

fn pascal_case(value: &str) -> String {
    value
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut characters = segment.chars();

            match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect(),
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(feature = "cli")]
fn path_string(path: &[&Ident], leaf: &Ident) -> String {
    path.iter()
        .copied()
        .chain(std::iter::once(leaf))
        .map(normalize_name)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests;
