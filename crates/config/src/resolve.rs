use std::borrow::Cow;
use std::collections::HashMap;

use crate::error::{ConfigError, ConfigErrorKind};
use crate::value::{ConfigStr, ConfigValue, Placeholder, Segment, lookup_path};

/// Maximum nesting depth for transitive placeholder resolution. Cycle detection
/// catches references that repeat; this caps a non-repeating *linear* chain so it
/// fails with a clear error instead of overflowing the stack.
const MAX_RESOLUTION_DEPTH: usize = 64;

/// Resolves a placeholder key to a raw string from one source (environment, an
/// in-memory overlay, ...).
///
/// Config-property-path resolution is deliberately *not* a `Resolver`: it must read
/// the same tree being deserialized, so it is handled inside [`ResolveCtx`].
pub trait Resolver: Send + Sync {
    /// Returns the raw value for `key`, or `None` if this source has none.
    fn resolve(&self, key: &str) -> Option<Cow<'_, str>>;
}

/// Resolves keys from process environment variables.
pub struct EnvResolver;

impl Resolver for EnvResolver {
    fn resolve(&self, key: &str) -> Option<Cow<'_, str>> {
        std::env::var(key).ok().map(Cow::Owned)
    }
}

/// An explicit in-memory overlay, primarily for deterministic, env-free tests.
pub struct MapResolver(pub HashMap<String, String>);

impl Resolver for MapResolver {
    fn resolve(&self, key: &str) -> Option<Cow<'_, str>> {
        self.0.get(key).map(|value| Cow::Borrowed(value.as_str()))
    }
}

/// An ordered set of resolvers, consulted left to right.
pub struct ResolverChain(pub Vec<Box<dyn Resolver>>);

impl ResolverChain {
    /// The default chain: environment variables only.
    pub fn env_default() -> Self {
        Self(vec![Box::new(EnvResolver)])
    }

    /// The first resolver in the chain that has a value for `key`.
    pub fn resolve(&self, key: &str) -> Option<Cow<'_, str>> {
        self.0.iter().find_map(|resolver| resolver.resolve(key))
    }
}

/// Per-run resolution context: the root tree (for property-path references), the
/// resolver chain, and an in-flight key stack for cycle detection.
///
/// Holds only borrows plus a small stack, so constructing one per deserialization
/// (including per hot-reload) is cheap and side-effect-free.
pub struct ResolveCtx<'cfg, 'r> {
    root: &'cfg ConfigValue,
    resolvers: &'r ResolverChain,
    in_flight: Vec<String>,
}

impl<'cfg, 'r> ResolveCtx<'cfg, 'r> {
    /// Creates a context over a root tree and resolver chain.
    pub fn new(root: &'cfg ConfigValue, resolvers: &'r ResolverChain) -> Self {
        Self {
            root,
            resolvers,
            in_flight: Vec::new(),
        }
    }

    /// Resolves one placeholder to its raw string, applying cycle detection, the
    /// dotted-path / uppercase-heuristic precedence, the inline default, and finally
    /// a missing-value error.
    pub(crate) fn resolve_placeholder(&mut self, p: &Placeholder) -> Result<String, ConfigError> {
        if self.in_flight.len() >= MAX_RESOLUTION_DEPTH {
            return Err(ConfigErrorKind::ResolutionDepthExceeded {
                limit: MAX_RESOLUTION_DEPTH,
            }
            .into());
        }

        if self.in_flight.iter().any(|key| key == &p.key) {
            return Err(ConfigErrorKind::ResolutionCycle {
                chain: self.in_flight.clone(),
                key: p.key.clone(),
            }
            .into());
        }

        let resolved = if p.key.contains('.') {
            self.resolve_path_then_env(&p.key)?
        } else if is_screaming(&p.key) {
            self.resolve_env_then_path(&p.key)?
        } else {
            self.resolve_path_then_env(&p.key)?
        };

        if let Some(value) = resolved {
            return Ok(value);
        }

        if let Some(default) = &p.default {
            return Ok(default.clone());
        }

        Err(ConfigErrorKind::MissingPlaceholder { key: p.key.clone() }.into())
    }

    /// Config path first, then the resolver chain (env). Used for dotted keys and
    /// for single-segment keys that do not look like an env var.
    fn resolve_path_then_env(&mut self, key: &str) -> Result<Option<String>, ConfigError> {
        if let Some(value) = self.resolve_config_path(key)? {
            return Ok(Some(value));
        }

        Ok(self.resolvers.resolve(key).map(Cow::into_owned))
    }

    /// Resolver chain (env) first, then the config tree. Used for single-segment
    /// keys that look like an env var (SCREAMING_SNAKE_CASE).
    fn resolve_env_then_path(&mut self, key: &str) -> Result<Option<String>, ConfigError> {
        if let Some(value) = self.resolvers.resolve(key) {
            return Ok(Some(value.into_owned()));
        }

        self.resolve_config_path(key)
    }

    /// Looks up `key` as a dotted path in the root tree and renders that node to a
    /// string, guarding against reference cycles. `None` when the path is absent.
    fn resolve_config_path(&mut self, key: &str) -> Result<Option<String>, ConfigError> {
        let node = match lookup_path(self.root, key) {
            Some(node) => node,
            None => return Ok(None),
        };

        self.in_flight.push(key.to_string());
        let rendered = render_node_as_string(node, self, key);
        self.in_flight.pop();

        rendered.map(Some)
    }
}

/// Renders a [`ConfigStr`] to a fully substituted string, resolving every
/// placeholder. This is the only path by which a templated/partial leaf produces a
/// value, and the depth-first point at which transitive references resolve.
pub(crate) fn render_str(
    s: &ConfigStr,
    ctx: &mut ResolveCtx<'_, '_>,
) -> Result<String, ConfigError> {
    let mut out = String::new();

    for segment in &s.segments {
        match segment {
            Segment::Literal(text) => out.push_str(text),

            Segment::Placeholder(p) => {
                let value = ctx.resolve_placeholder(p)?;

                out.push_str(&value);
            }
        }
    }

    Ok(out)
}

/// Renders a referenced tree node to a string. Scalars format via `Display`; a
/// string leaf renders recursively (hence the cycle guard); arrays, tables, and null
/// cannot be spliced into a string and error as `NotStringRenderable`.
fn render_node_as_string(
    node: &ConfigValue,
    ctx: &mut ResolveCtx<'_, '_>,
    key: &str,
) -> Result<String, ConfigError> {
    match node {
        ConfigValue::Str(s) => render_str(s, ctx),
        ConfigValue::Bool(b) => Ok(b.to_string()),
        ConfigValue::Int(n) => Ok(n.to_string()),
        ConfigValue::Float(f) => Ok(f.to_string()),

        ConfigValue::Null | ConfigValue::Array(_) | ConfigValue::Table(_) => {
            Err(ConfigErrorKind::NotStringRenderable {
                key: key.to_string(),
            }
            .into())
        }
    }
}

/// Whether a single-segment key looks like an environment variable: at least one
/// uppercase ASCII letter and no lowercase ones (SCREAMING_SNAKE_CASE).
fn is_screaming(key: &str) -> bool {
    let has_upper = key.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = key.chars().any(|c| c.is_ascii_lowercase());

    has_upper && !has_lower
}
