use std::iter::Peekable;
use std::str::Chars;

use crate::error::{TemplateError, TemplateErrorKind};
use crate::value::{Placeholder, Segment, StrKind};

/// Parses a raw source string into placeholder segments, classified for the
/// deserializer.
///
/// Grammar (single pass, no regex): `${key}` and `${key:default}` are placeholders;
/// `$$` collapses to a literal `$` (so `$${X}` is the literal text `${X}`); a lone
/// `$` not followed by `{` is a literal `$`. An unterminated `${` is an error.
/// Nesting and recursive defaults are out of scope in v1.
pub(crate) fn parse_template(raw: &str) -> Result<(Vec<Segment>, StrKind), TemplateError> {
    let mut segments: Vec<Segment> = Vec::new();
    let mut literal = String::new();
    let mut chars = raw.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '$' {
            literal.push(c);

            continue;
        }

        match chars.peek() {
            Some('$') => {
                literal.push('$');
                chars.next();
            }

            Some('{') => {
                chars.next();
                let placeholder = parse_placeholder(&mut chars)?;

                if !literal.is_empty() {
                    segments.push(Segment::Literal(std::mem::take(&mut literal)));
                }

                segments.push(Segment::Placeholder(placeholder));
            }

            _ => {
                literal.push('$');
            }
        }
    }

    if !literal.is_empty() {
        segments.push(Segment::Literal(literal));
    }

    let kind = classify(&segments);

    Ok((segments, kind))
}

/// Reads the body of a placeholder after the opening `${`, up to and including the
/// closing `}`. The key runs to the first `:` (which begins the default) or `}`.
fn parse_placeholder(chars: &mut Peekable<Chars<'_>>) -> Result<Placeholder, TemplateError> {
    let mut key = String::new();

    loop {
        let c = chars
            .next()
            .ok_or(TemplateErrorKind::UnterminatedPlaceholder)?;

        if c == '}' {
            return Ok(Placeholder { key, default: None });
        }

        if c == ':' {
            let default = read_default(chars)?;

            return Ok(Placeholder {
                key,
                default: Some(default),
            });
        }

        key.push(c);
    }
}

/// Reads the inline default after a `:`, up to the closing `}`.
fn read_default(chars: &mut Peekable<Chars<'_>>) -> Result<String, TemplateError> {
    let mut default = String::new();

    loop {
        let c = chars
            .next()
            .ok_or(TemplateErrorKind::UnterminatedPlaceholder)?;

        if c == '}' {
            break;
        }

        default.push(c);
    }

    Ok(default)
}

/// Classifies parsed segments. An empty or single-literal string is `Literal`; a
/// lone placeholder is `FullPlaceholder`; anything else is `Templated`.
fn classify(segments: &[Segment]) -> StrKind {
    match segments {
        [] => StrKind::Literal,
        [Segment::Literal(_)] => StrKind::Literal,
        [Segment::Placeholder(_)] => StrKind::FullPlaceholder,
        _ => StrKind::Templated,
    }
}
