use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use overseerd_config::{ConfigValue, DefaultSpec, Resolver, ResolverChain, from_value_in};
use serde::de::DeserializeOwned;
use tracing::{debug, info, instrument, trace};

use crate::dirs::{Config, Dir, DirectoriesManager};

use super::{ConfigError, ConfigProperties};

/// A parser from source text to the normalized config tree.
type Parser = fn(&str) -> Result<ConfigValue, overseerd_config::ConfigError>;

/// Which source format(s) a [`ConfigManager`] reads. Retained as runtime data so the
/// daemon knows how to re-read on reload even after the `Format` type is erased.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatId {
    Toml,
    Yaml,
    Dynamic,
}

/// A config source format: the file extensions it owns (and their parsers), in
/// precedence order. The type parameter on [`ConfigManager`] is what makes
/// `ConfigManager<Toml>` only try `.toml` while `ConfigManager<Dynamic>` tries all.
pub trait Format: 'static {
    /// The runtime tag recorded on a manager built with this format.
    const ID: FormatId;

    /// The `(extension, parser)` pairs this format loads, highest precedence last
    /// (so later files override earlier on merge).
    fn parsers() -> Vec<(&'static str, Parser)>;
}

/// Reads only TOML (`application.toml`).
pub struct Toml;

impl Format for Toml {
    const ID: FormatId = FormatId::Toml;

    fn parsers() -> Vec<(&'static str, Parser)> {
        vec![("toml", overseerd_config::format::toml::from_str)]
    }
}

/// Reads only YAML (`application.yaml` / `.yml`).
#[cfg(feature = "yaml")]
pub struct Yaml;

#[cfg(feature = "yaml")]
impl Format for Yaml {
    const ID: FormatId = FormatId::Yaml;

    fn parsers() -> Vec<(&'static str, Parser)> {
        vec![
            ("yaml", overseerd_config::format::yaml::from_str),
            ("yml", overseerd_config::format::yaml::from_str),
        ]
    }
}

/// Reads every enabled format, letting the file extension decide the parser.
pub struct Dynamic;

impl Format for Dynamic {
    const ID: FormatId = FormatId::Dynamic;

    fn parsers() -> Vec<(&'static str, Parser)> {
        #[allow(unused_mut)]
        let mut parsers: Vec<(&'static str, Parser)> =
            vec![("toml", overseerd_config::format::toml::from_str)];

        #[cfg(feature = "yaml")]
        {
            parsers.push(("yaml", overseerd_config::format::yaml::from_str));
            parsers.push(("yml", overseerd_config::format::yaml::from_str));
        }

        parsers
    }
}

/// The merged configuration tree, owning the format it was read in.
///
/// Built by the application (typically in `main`) so values can configure the
/// transport *before* the daemon exists, then handed to
/// [`DaemonBuilder::config_source`](crate::DaemonBuilder::config_source) to seed the
/// dependency-injection bindings. The `Format` type optimizes *what gets loaded*; it
/// is irrelevant once the tree is parsed, so the DI path uses the format-erased
/// [`ConfigManager<Dynamic>`]. The original format is retained as runtime data
/// ([`format`](Self::format)) so reload still knows how to re-read.
pub struct ConfigManager<F = Dynamic> {
    root: ConfigValue,
    resolvers: ResolverChain,
    format: FormatId,
    sources: Vec<PathBuf>,
    _marker: PhantomData<F>,
}

impl<F: Format> ConfigManager<F> {
    /// An empty configuration, for daemons that bind no config. Reads no files.
    pub fn empty() -> Self {
        Self::wrap(ConfigValue::Table(Vec::new()), Vec::new())
    }

    /// Parses one in-memory document in this format (the first parser wins for
    /// multi-extension formats). Handy for tests and embedded defaults.
    ///
    /// Named `from_str` to read as `ConfigManager::<Toml>::from_str`; it is a typed,
    /// fallible constructor, not the `std::str::FromStr` trait method.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(text: &str) -> Result<Self, ConfigError> {
        let parsers = F::parsers();
        let (_, parse) = parsers
            .first()
            .expect("a format defines at least one parser");

        let root = parse(text).map_err(|source| ConfigError::Parse {
            path: PathBuf::from("<in-memory>"),
            source,
        })?;

        Ok(Self::wrap(root, Vec::new()))
    }

    /// Discovers and deep-merges config from the directory `dir` (typically a
    /// `Dir<Config>` from the [`DirectoriesManager`](crate::dirs::DirectoriesManager)).
    ///
    /// The base `application.<ext>` underlies the per-profile overlays
    /// `application-<profile>.<ext>`, each overriding the previous. Profiles come from
    /// `OVERSEERD_PROFILES` (comma-separated) first, then `profiles`. A missing file is
    /// skipped; a malformed one is an error.
    #[instrument(target = "overseerd::config", level = "debug", skip(dir, profiles), fields(dir = %dir.path().display()))]
    pub fn load_in(dir: &Dir<Config>, profiles: &[String]) -> Result<Self, ConfigError> {
        let parsers = F::parsers();
        let active = resolve_profiles(profiles);

        let mut root = ConfigValue::Table(Vec::new());
        let mut sources = Vec::new();

        debug!(target: "overseerd::config", profiles = ?active, "loading config");

        merge_stem(&mut root, dir.path(), "application", &parsers, &mut sources)?;

        for profile in &active {
            let stem = format!("application-{profile}");

            merge_stem(&mut root, dir.path(), &stem, &parsers, &mut sources)?;
        }

        info!(target: "overseerd::config", sources = sources.len(), profiles = active.len(), "config loaded");

        Ok(Self::wrap(root, sources))
    }

    /// Loads and merges config from `directories`' config directory **with the `${@kind}`
    /// directory namespace registered**, so values can reference `${@runtime}`, `${@data}`,
    /// and friends.
    ///
    /// The one-call equivalent of
    /// [`load_in`](Self::load_in)`(&directories.dir::<Config>(), profiles)` followed by
    /// [`with_directories`](Self::with_directories)`(directories)`. Prefer this over
    /// `load_in` whenever a [`DirectoriesManager`] is on hand — `load_in` only receives the
    /// config directory, so it cannot wire the namespace itself.
    pub fn load_from(
        directories: &DirectoriesManager,
        profiles: &[String],
    ) -> Result<Self, ConfigError> {
        let manager = Self::load_in(&directories.dir::<Config>(), profiles)?;

        Ok(manager.with_directories(directories))
    }

    fn wrap(root: ConfigValue, sources: Vec<PathBuf>) -> Self {
        Self {
            root,
            resolvers: ResolverChain::env_default(),
            format: F::ID,
            sources,
            _marker: PhantomData,
        }
    }
}

impl<F> ConfigManager<F> {
    /// Deserializes the subtree at `path` into `T`, resolving `${...}` placeholders
    /// against environment variables and other config paths. The single entry point
    /// shared by transport setup in `main` and DI-seeded `Cfg<T>` injection.
    #[instrument(target = "overseerd::config", level = "debug", skip(self))]
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ConfigError> {
        let subtree = self
            .root
            .get_path(path)
            .ok_or_else(|| ConfigError::MissingPath {
                path: path.to_string(),
            })?;

        // Deserialize the subtree, but resolve placeholders against the full tree so
        // absolute property-path references (`${app.server.port}`) still resolve.
        let value = from_value_in(&self.root, subtree, &self.resolvers).map_err(|source| {
            ConfigError::Substitution {
                path: path.to_string(),
                source,
            }
        })?;

        trace!(target: "overseerd::config", "config subtree deserialized");

        Ok(value)
    }

    /// Like [`get`](Self::get), but for a [`ConfigProperties`] type: its `#[default = ".."]`
    /// field defaults are merged *under* the file values before deserializing, so a missing
    /// field falls back to its (possibly templated) default and resolves through the normal
    /// `${...}` pipeline.
    ///
    /// When the `path` subtree is absent and the type declares defaults, deserialization
    /// proceeds from an empty table so a fully-defaulted type still materializes; absent and
    /// default-free remains a [`MissingPath`](ConfigError::MissingPath) error, matching
    /// [`get`](Self::get).
    #[instrument(target = "overseerd::config", level = "debug", skip(self))]
    pub fn get_config<T: ConfigProperties>(&self, path: &str) -> Result<T, ConfigError> {
        let defaults = T::defaults();

        let mut subtree = match self.root.get_path(path) {
            Some(node) => node.clone(),

            None => {
                if matches!(defaults, DefaultSpec::None) {
                    return Err(ConfigError::MissingPath {
                        path: path.to_string(),
                    });
                }

                ConfigValue::Table(Vec::new())
            }
        };

        defaults
            .fill_missing(&mut subtree)
            .map_err(|source| ConfigError::Substitution {
                path: path.to_string(),
                source,
            })?;

        let value = from_value_in(&self.root, &subtree, &self.resolvers).map_err(|source| {
            ConfigError::Substitution {
                path: path.to_string(),
                source,
            }
        })?;

        trace!(target: "overseerd::config", "config subtree deserialized with defaults");

        Ok(value)
    }

    /// Appends a [`Resolver`] to the chain consulted during placeholder substitution. Later
    /// resolvers are tried only when earlier ones (env by default) have no value.
    pub fn with_resolver(mut self, resolver: Box<dyn Resolver>) -> Self {
        self.resolvers.0.push(resolver);

        self
    }

    /// Registers the directory namespace, so config values may reference application
    /// directories as `${@runtime}`, `${@config}`, `${@data}`, `${@cache}`, `${@state}`,
    /// and `${@tmp}` (e.g. `socket = "${@runtime}/app.sock"`).
    pub fn with_directories(self, directories: &DirectoriesManager) -> Self {
        self.with_resolver(Box::new(directories.resolver()))
    }

    /// Whether `path` resolves to a present subtree.
    pub fn contains(&self, path: &str) -> bool {
        self.root.get_path(path).is_some()
    }

    /// The format this manager was read in (retained even after erasure, so reload
    /// knows how to re-read).
    pub fn format(&self) -> FormatId {
        self.format
    }

    /// The files this manager was loaded from, in merge order — the inputs a future
    /// reload re-reads.
    pub fn sources(&self) -> &[PathBuf] {
        &self.sources
    }

    /// Erases the `Format` type, keeping the runtime format tag and sources. The DI
    /// path stores managers in this form, since the tree is already parsed.
    pub fn into_dynamic(self) -> ConfigManager<Dynamic> {
        ConfigManager {
            root: self.root,
            resolvers: self.resolvers,
            format: self.format,
            sources: self.sources,
            _marker: PhantomData,
        }
    }
}

/// Combines `OVERSEERD_PROFILES` (consulted first) with the explicitly supplied
/// profiles, preserving order.
fn resolve_profiles(explicit: &[String]) -> Vec<String> {
    let mut profiles = Vec::new();

    if let Ok(env) = std::env::var("OVERSEERD_PROFILES") {
        let from_env = env
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        profiles.extend(from_env);
    }

    profiles.extend(explicit.iter().cloned());

    profiles
}

/// Merges every present `stem.<ext>` file under `dir` into `root`, in parser order.
fn merge_stem(
    root: &mut ConfigValue,
    dir: &Path,
    stem: &str,
    parsers: &[(&'static str, Parser)],
    sources: &mut Vec<PathBuf>,
) -> Result<(), ConfigError> {
    for (ext, parse) in parsers {
        let path = dir.join(format!("{stem}.{ext}"));

        if !path.exists() {
            trace!(target: "overseerd::config", path = %path.display(), "config file absent, skipping");

            continue;
        }

        let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::Io {
            path: path.clone(),
            source,
        })?;
        let parsed = parse(&text).map_err(|source| ConfigError::Parse {
            path: path.clone(),
            source,
        })?;

        merge_into(root, parsed);
        sources.push(path.clone());

        debug!(target: "overseerd::config", path = %path.display(), "merged config file");
    }

    Ok(())
}

/// Deep-merges `overlay` into `base`: tables recurse key-by-key, every other value
/// (scalar or array) replaces the base value wholesale.
fn merge_into(base: &mut ConfigValue, overlay: ConfigValue) {
    let ConfigValue::Table(overlay_entries) = overlay else {
        *base = overlay;
        return;
    };

    let ConfigValue::Table(base_entries) = base else {
        *base = ConfigValue::Table(overlay_entries);
        return;
    };

    for (key, value) in overlay_entries {
        match base_entries.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => merge_into(&mut slot.1, value),
            None => base_entries.push((key, value)),
        }
    }
}
