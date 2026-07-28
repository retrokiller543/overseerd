use std::collections::HashMap;

use proc_macro2::Span;
use syn::meta::ParseNestedMeta;
use syn::parse::Parse as _;
use syn::{Attribute, Expr, ExprArray, ExprLit, Lit, LitStr, Token};

use super::{normalize_name, variant_ident};
use crate::app::model::{CommandEntry, CommandEntryKind};
use crate::app::policy::{ArgumentPolicy, CliPolicy};

/// Canonical owner of one statically known parser claim.
#[derive(Clone)]
enum ClaimOwner {
    Framework(&'static str),
    Application(String),
}

impl std::fmt::Display for ClaimOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Framework(slot) => write!(formatter, "framework reserved slot `{slot}`"),
            Self::Application(command) => write!(formatter, "application command `{command}`"),
        }
    }
}

/// Literal claims visible within one command parser scope.
#[derive(Clone, Default)]
struct LiteralClaims {
    commands: HashMap<String, Claim>,
    longs: HashMap<String, Claim>,
    shorts: HashMap<char, Claim>,
}

/// One parser claim with enough provenance to prefer user spans in diagnostics.
#[derive(Clone)]
struct Claim {
    owner: ClaimOwner,
    span: Span,
    generated: bool,
}

/// Literal parser metadata extracted from an application command declaration.
#[derive(Default)]
struct CommandMetadata {
    aliases: Vec<(String, Span)>,
    longs: Vec<(String, Span)>,
    shorts: Vec<(char, Span)>,
}

/// Rejects collisions whose complete parser-visible values are known to the macro.
pub(crate) fn validate_literal_collisions(
    commands: &[CommandEntry],
    policy: &CliPolicy,
    has_serve_phase: bool,
) -> syn::Result<()> {
    let mut global = LiteralClaims::default();

    insert_argument_claims(
        &mut global,
        "config",
        policy.config.as_ref().map(|value| &value.value),
        "config",
        Some('c'),
    )?;
    insert_argument_claims(
        &mut global,
        "profile",
        policy.profile.as_ref().map(|value| &value.value),
        "profile",
        Some('p'),
    )?;
    insert_argument_claims(
        &mut global,
        "log",
        policy.log.as_ref().map(|value| &value.value),
        "log",
        None,
    )?;
    insert_argument_claims(
        &mut global,
        "log-format",
        policy
            .log_format
            .as_ref()
            .map(|value| &value.value.argument),
        "log-format",
        None,
    )?;
    insert_argument_claims(
        &mut global,
        "color",
        policy.color.as_ref().map(|value| &value.value.argument),
        "color",
        None,
    )?;

    let mut root = LiteralClaims::default();

    insert_built_ins(&mut root, true)?;
    merge_claims(&mut root, &global)?;
    insert_serve_claims(&mut root, policy, has_serve_phase)?;

    validate_commands(
        commands,
        &root,
        &global,
        &[],
        has_serve_phase
            && policy
                .serve
                .as_ref()
                .is_none_or(|serve| serve.value.enabled),
    )
}

fn insert_argument_claims(
    claims: &mut LiteralClaims,
    owner: &'static str,
    policy: Option<&ArgumentPolicy>,
    default_name: &'static str,
    default_short: Option<char>,
) -> syn::Result<()> {
    if policy.is_some_and(|policy| !policy.enabled) {
        return Ok(());
    }

    let claim_owner = ClaimOwner::Framework(owner);
    let name = policy.and_then(|policy| policy.name.as_ref());
    let name_value = name
        .map(LitStr::value)
        .unwrap_or_else(|| default_name.to_string());
    let name_span = name.map(LitStr::span).unwrap_or_else(Span::call_site);

    insert_claim(
        &mut claims.longs,
        name_value,
        name_span,
        claim_owner.clone(),
        "long option",
        name.is_none(),
    )?;

    if let Some(policy) = policy {
        for alias in policy.aliases.iter().chain(&policy.visible_aliases) {
            insert_long(claims, alias.value(), alias.span(), claim_owner.clone())?;
        }
    }

    let short = match policy.and_then(|policy| policy.short.as_ref()) {
        Some(short) => short.as_ref().map(|short| (short.value(), short.span())),
        None => default_short.map(|short| (short, Span::call_site())),
    };

    if let Some((short, span)) = short {
        insert_claim(
            &mut claims.shorts,
            short,
            span,
            claim_owner,
            "short option",
            policy.and_then(|policy| policy.short.as_ref()).is_none(),
        )?;
    }

    Ok(())
}

fn insert_built_ins(claims: &mut LiteralClaims, include_version: bool) -> syn::Result<()> {
    let help = ClaimOwner::Framework("help");

    insert_generated_long(
        claims,
        String::from("help"),
        Span::call_site(),
        help.clone(),
    )?;
    insert_generated_short(claims, 'h', Span::call_site(), help.clone())?;
    insert_generated_command(claims, String::from("help"), Span::call_site(), help)?;

    if include_version {
        let version = ClaimOwner::Framework("version");

        insert_generated_long(
            claims,
            String::from("version"),
            Span::call_site(),
            version.clone(),
        )?;
        insert_generated_short(claims, 'V', Span::call_site(), version)?;
    }

    Ok(())
}

fn insert_serve_claims(
    claims: &mut LiteralClaims,
    policy: &CliPolicy,
    has_serve_phase: bool,
) -> syn::Result<()> {
    let Some(serve) = policy.serve.as_ref() else {
        if has_serve_phase {
            insert_generated_command(
                claims,
                String::from("serve"),
                Span::call_site(),
                ClaimOwner::Framework("serve"),
            )?;
        }

        return Ok(());
    };

    let serve = &serve.value;

    if !has_serve_phase || !serve.enabled {
        return Ok(());
    }

    let owner = ClaimOwner::Framework("serve");
    let name = serve.name.as_ref();

    let name_value = name
        .map(LitStr::value)
        .unwrap_or_else(|| String::from("serve"));
    let name_span = name.map(LitStr::span).unwrap_or_else(Span::call_site);

    insert_claim(
        &mut claims.commands,
        name_value,
        name_span,
        owner.clone(),
        "command name or alias",
        name.is_none(),
    )?;

    for alias in serve.aliases.iter().chain(&serve.visible_aliases) {
        insert_command(claims, alias.value(), alias.span(), owner.clone())?;
    }

    Ok(())
}

fn validate_commands(
    entries: &[CommandEntry],
    scope: &LiteralClaims,
    global: &LiteralClaims,
    parent: &[String],
    reserve_serve_variant: bool,
) -> syn::Result<()> {
    let mut claims = scope.clone();

    for entry in entries {
        let name = normalize_name(&entry.name);
        let mut path = parent.to_vec();

        path.push(name.clone());

        let owner = ClaimOwner::Application(path.join(" "));
        let metadata = command_metadata(&entry.attributes)?;

        insert_command(&mut claims, name, entry.name.span(), owner.clone())?;

        if parent.is_empty() && reserve_serve_variant && variant_ident(&entry.name) == "Serve" {
            return Err(syn::Error::new(
                entry.name.span(),
                format!(
                    "generated Rust variant `Serve` is claimed by {} and {owner}",
                    ClaimOwner::Framework("serve")
                ),
            ));
        }

        for (alias, span) in metadata.aliases {
            insert_command(&mut claims, alias, span, owner.clone())?;
        }

        for (long, span) in metadata.longs {
            insert_long(&mut claims, long, span, owner.clone())?;
        }

        for (short, span) in metadata.shorts {
            insert_short(&mut claims, short, span, owner.clone())?;
        }

        if let CommandEntryKind::Namespace(children) = &entry.kind {
            let mut nested = LiteralClaims::default();

            insert_built_ins(&mut nested, false)?;
            merge_claims(&mut nested, global)?;
            validate_commands(children, &nested, global, &path, false)?;
        }
    }

    Ok(())
}

fn command_metadata(attributes: &[Attribute]) -> syn::Result<CommandMetadata> {
    let mut metadata = CommandMetadata::default();

    for attribute in attributes {
        if !attribute.path().is_ident("command") {
            continue;
        }

        attribute.parse_nested_meta(|meta| collect_command_metadata(meta, &mut metadata))?;
    }

    Ok(metadata)
}

fn collect_command_metadata(
    meta: ParseNestedMeta<'_>,
    metadata: &mut CommandMetadata,
) -> syn::Result<()> {
    let name = meta
        .path
        .get_ident()
        .map(ToString::to_string)
        .unwrap_or_default();
    let expressions = parse_metadata_expressions(&meta)?;

    match name.as_str() {
        "alias" | "aliases" | "visible_alias" | "visible_aliases" => {
            collect_strings(&expressions, &mut metadata.aliases);
        }
        "long_flag"
        | "long_flag_alias"
        | "long_flag_aliases"
        | "visible_long_flag_alias"
        | "visible_long_flag_aliases" => {
            collect_strings(&expressions, &mut metadata.longs);
        }
        "short_flag"
        | "short_flag_alias"
        | "short_flag_aliases"
        | "visible_short_flag_alias"
        | "visible_short_flag_aliases" => {
            collect_chars(&expressions, &mut metadata.shorts);
        }
        _ => {}
    }

    Ok(())
}

fn parse_metadata_expressions(meta: &ParseNestedMeta<'_>) -> syn::Result<Vec<Expr>> {
    if meta.input.peek(Token![=]) {
        return Ok(vec![meta.value()?.parse()?]);
    }

    if meta.input.peek(syn::token::Paren) {
        let content;

        syn::parenthesized!(content in meta.input);

        return Ok(content
            .parse_terminated(Expr::parse, Token![,])?
            .into_iter()
            .collect());
    }

    Ok(Vec::new())
}

fn collect_strings<'a>(
    expressions: impl IntoIterator<Item = &'a Expr>,
    values: &mut Vec<(String, Span)>,
) {
    for expression in expressions {
        match expression {
            Expr::Lit(ExprLit {
                lit: Lit::Str(value),
                ..
            }) => values.push((value.value(), value.span())),
            Expr::Array(ExprArray { elems, .. }) => collect_strings(elems, values),
            _ => {}
        }
    }
}

fn collect_chars<'a>(
    expressions: impl IntoIterator<Item = &'a Expr>,
    values: &mut Vec<(char, Span)>,
) {
    for expression in expressions {
        match expression {
            Expr::Lit(ExprLit {
                lit: Lit::Char(value),
                ..
            }) => values.push((value.value(), value.span())),
            Expr::Array(ExprArray { elems, .. }) => collect_chars(elems, values),
            _ => {}
        }
    }
}

fn insert_command(
    claims: &mut LiteralClaims,
    value: String,
    span: Span,
    owner: ClaimOwner,
) -> syn::Result<()> {
    insert_claim(
        &mut claims.commands,
        value,
        span,
        owner,
        "command name or alias",
        false,
    )
}

fn insert_long(
    claims: &mut LiteralClaims,
    value: String,
    span: Span,
    owner: ClaimOwner,
) -> syn::Result<()> {
    insert_claim(&mut claims.longs, value, span, owner, "long option", false)
}

fn insert_short(
    claims: &mut LiteralClaims,
    value: char,
    span: Span,
    owner: ClaimOwner,
) -> syn::Result<()> {
    insert_claim(
        &mut claims.shorts,
        value,
        span,
        owner,
        "short option",
        false,
    )
}

fn insert_claim<T>(
    claims: &mut HashMap<T, Claim>,
    value: T,
    span: Span,
    owner: ClaimOwner,
    kind: &str,
    generated: bool,
) -> syn::Result<()>
where
    T: Eq + std::hash::Hash + std::fmt::Display,
{
    if let Some(first) = claims.get(&value) {
        let span = if first.generated && !generated {
            span
        } else if !first.generated && generated {
            first.span
        } else {
            span
        };

        return Err(syn::Error::new(
            span,
            format!("{kind} `{value}` is claimed by {} and {owner}", first.owner),
        ));
    }

    claims.insert(
        value,
        Claim {
            owner,
            span,
            generated,
        },
    );

    Ok(())
}

fn merge_claims(target: &mut LiteralClaims, source: &LiteralClaims) -> syn::Result<()> {
    for (value, claim) in &source.commands {
        merge_claim(
            &mut target.commands,
            value.clone(),
            claim,
            "command name or alias",
        )?;
    }

    for (value, claim) in &source.longs {
        merge_claim(&mut target.longs, value.clone(), claim, "long option")?;
    }

    for (value, claim) in &source.shorts {
        merge_claim(&mut target.shorts, *value, claim, "short option")?;
    }

    Ok(())
}

fn merge_claim<T>(
    claims: &mut HashMap<T, Claim>,
    value: T,
    claim: &Claim,
    kind: &str,
) -> syn::Result<()>
where
    T: Eq + std::hash::Hash + std::fmt::Display,
{
    if let Some(existing) = claims.get(&value) {
        let span = if existing.generated && !claim.generated {
            claim.span
        } else if !existing.generated && claim.generated {
            existing.span
        } else {
            claim.span
        };

        return Err(syn::Error::new(
            span,
            format!(
                "{kind} `{value}` is claimed by {} and {}",
                claim.owner, existing.owner
            ),
        ));
    }

    claims.insert(value, claim.clone());

    Ok(())
}

fn insert_generated_command(
    claims: &mut LiteralClaims,
    value: String,
    span: Span,
    owner: ClaimOwner,
) -> syn::Result<()> {
    insert_claim(
        &mut claims.commands,
        value,
        span,
        owner,
        "command name or alias",
        true,
    )
}

fn insert_generated_long(
    claims: &mut LiteralClaims,
    value: String,
    span: Span,
    owner: ClaimOwner,
) -> syn::Result<()> {
    insert_claim(&mut claims.longs, value, span, owner, "long option", true)
}

fn insert_generated_short(
    claims: &mut LiteralClaims,
    value: char,
    span: Span,
    owner: ClaimOwner,
) -> syn::Result<()> {
    insert_claim(&mut claims.shorts, value, span, owner, "short option", true)
}
