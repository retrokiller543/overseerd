use std::io;
use std::sync::OnceLock;

use cargo_upwell::{
    BuiltInRenderer, ComponentRenderRequest, ComponentRendererHost, RendererImplementation,
    RendererRegistry, ResolvedRenderer,
};

pub(crate) fn registry() -> &'static RendererRegistry {
    static REGISTRY: OnceLock<RendererRegistry> = OnceLock::new();

    REGISTRY.get_or_init(|| {
        RendererRegistry::load().unwrap_or_else(|error| {
            let error = terminal_safe_text(&error.to_string(), 2048);
            eprintln!("cargo upwell could not load the renderer catalog: {error}");
            eprintln!("cargo upwell is continuing with built-in renderer formats");

            RendererRegistry::builtins()
        })
    })
}

pub(crate) fn format_registry() -> &'static RendererRegistry {
    registry()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_selected(
    selected: ResolvedRenderer<'_>,
    fallback: Option<ResolvedRenderer<'_>>,
    tooling_schema: &semver::Version,
    resources: &[String],
    color: bool,
    terminal: bool,
    payload: io::Result<Vec<u8>>,
    native: impl FnOnce(BuiltInRenderer, &mut dyn io::Write) -> io::Result<()>,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    match selected.descriptor().implementation() {
        RendererImplementation::Native(renderer) => native(*renderer, output),
        RendererImplementation::Component(component) => {
            let payload = payload?;
            let request = ComponentRenderRequest {
                command: selected.format().command(),
                format: selected.format().id(),
                media_type: selected.format().media_type(),
                tooling_schema,
                resources,
                color: color && selected.format().capabilities().color,
                payload: &payload,
            };

            let rendered = component_host().and_then(|host| {
                host.render(component, request)
                    .map_err(|error| error.to_string())
            });
            match rendered {
                Ok(rendered) if terminal && component.utf8() => {
                    write_terminal_safe(&rendered, request.color, output)
                }
                Ok(rendered) => output.write_all(&rendered),
                Err(error) => {
                    let error = terminal_safe_text(&error.to_string(), 2048);
                    eprintln!(
                        "cargo upwell renderer `{}` failed: {error}",
                        selected.descriptor().id()
                    );

                    let Some(fallback) = fallback else {
                        return Err(io::Error::other(error));
                    };
                    let RendererImplementation::Native(renderer) =
                        fallback.descriptor().implementation()
                    else {
                        unreachable!("native fallback is native")
                    };

                    eprintln!(
                        "cargo upwell is falling back to `{}`",
                        fallback.descriptor().id()
                    );
                    native(*renderer, output)
                }
            }
        }
    }
}

fn write_terminal_safe(rendered: &[u8], color: bool, output: &mut dyn io::Write) -> io::Result<()> {
    let mut index = 0;
    while index < rendered.len() {
        if color
            && rendered[index] == 0x1b
            && let Some(length) = sgr_length(&rendered[index..])
        {
            output.write_all(&rendered[index..index + length])?;
            index += length;
            continue;
        }

        if let Some((character, length)) = next_utf8_character(&rendered[index..])
            && length > 1
        {
            if character.is_control() || is_bidi_formatting(character) {
                write!(output, "\\u{{{:x}}}", character as u32)?;
            } else {
                output.write_all(&rendered[index..index + length])?;
            }
            index += length;
            continue;
        }

        match rendered[index] {
            b'\n' | b'\t' => output.write_all(&rendered[index..=index])?,
            0x00..=0x1f | 0x7f..=0x9f => write!(output, "\\x{:02x}", rendered[index])?,
            _ => output.write_all(&rendered[index..=index])?,
        }
        index += 1;
    }

    Ok(())
}

fn is_bidi_formatting(character: char) -> bool {
    matches!(
        character,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

fn sgr_length(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 3 || bytes[0..2] != [0x1b, b'['] {
        return None;
    }

    for (offset, byte) in bytes[2..bytes.len().min(34)].iter().enumerate() {
        if *byte == b'm' {
            return Some(offset + 3);
        }
        if !byte.is_ascii_digit() && *byte != b';' {
            return None;
        }
    }

    None
}

fn next_utf8_character(bytes: &[u8]) -> Option<(char, usize)> {
    let first = *bytes.first()?;
    let length = match first {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let encoded = bytes.get(..length)?;
    let character = std::str::from_utf8(encoded).ok()?.chars().next()?;

    Some((character, length))
}

fn terminal_safe_text(value: &str, limit: usize) -> String {
    let mut output = Vec::new();
    let bytes = value.as_bytes();
    let truncated = bytes.len() > limit;
    let bytes = &bytes[..bytes.len().min(limit)];

    write_terminal_safe(bytes, false, &mut output).expect("writing to a vector cannot fail");
    if truncated {
        output.extend_from_slice(b"...");
    }

    String::from_utf8_lossy(&output).into_owned()
}

fn component_host() -> Result<&'static ComponentRendererHost, String> {
    static HOST: OnceLock<Result<ComponentRendererHost, String>> = OnceLock::new();

    HOST.get_or_init(|| {
        ComponentRendererHost::new(Default::default()).map_err(|error| error.to_string())
    })
    .as_ref()
    .map_err(Clone::clone)
}

#[cfg(test)]
pub(crate) fn write_component_output_for_test(
    rendered: &[u8],
    terminal: bool,
    color: bool,
) -> Vec<u8> {
    let mut output = Vec::new();

    if terminal {
        write_terminal_safe(rendered, color, &mut output).expect("vector writes cannot fail");
    } else {
        output.extend_from_slice(rendered);
    }

    output
}

#[cfg(test)]
mod tests;
