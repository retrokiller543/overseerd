use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::Deserialize;
use thiserror::Error;

const CATALOG_SCHEMA: &str = "1";

/// Kind of project produced by one template.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TemplateKind {
    /// Standalone application crate.
    Application,
    /// Application workspace with a thin application member.
    ApplicationWorkspace,
    /// Protocol-neutral plugin library crate.
    Plugin,
    /// First-class protocol library crate.
    Protocol,
}

/// One future external developer-tool definition retained in the shared catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolEntry {
    id: String,
    command: String,
    description: String,
    package: Option<String>,
    protocols: Vec<String>,
}

impl ToolEntry {
    /// Stable catalog identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Exact executable name exposed by the tool.
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Human-readable tool summary.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Optional Cargo package used by a future managed installer.
    pub fn package(&self) -> Option<&str> {
        self.package.as_deref()
    }

    /// Protocol identities whose documents this tool understands.
    pub fn protocols(&self) -> &[String] {
        &self.protocols
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum CatalogEntry {
    /// Locally referenced cargo-generate template.
    Template {
        /// Stable catalog identity.
        id: String,
        /// Local template directory, relative to the catalog file when not absolute.
        path: PathBuf,
        /// Human-readable template summary.
        #[serde(default)]
        description: String,
    },
    /// Separately installed developer tool. Installation is intentionally not performed by init.
    Tool {
        /// Stable catalog identity.
        id: String,
        /// Exact executable name.
        command: String,
        /// Human-readable tool summary.
        #[serde(default)]
        description: String,
        /// Optional Cargo package for a future managed installer.
        #[serde(default)]
        package: Option<String>,
        /// Supported protocol identities.
        #[serde(default)]
        protocols: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogDocument {
    schema: String,
    #[serde(default)]
    entries: Vec<CatalogEntry>,
}

/// Effective built-in plus user-controlled template and tool catalog.
#[derive(Clone, Debug)]
pub struct Catalog {
    templates: BTreeMap<String, TemplateEntry>,
    tools: BTreeMap<String, ToolEntry>,
}

#[derive(Clone, Debug)]
pub(crate) struct TemplateEntry {
    kind: TemplateKind,
    path: Option<PathBuf>,
    description: String,
}

impl TemplateEntry {
    pub(crate) const fn kind(&self) -> TemplateKind {
        self.kind
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

impl Catalog {
    /// Loads built-ins and overlays an explicit or platform-default user catalog by stable ID.
    pub fn load(path: Option<&Path>) -> Result<Self, CatalogError> {
        let mut catalog = Self::builtins();
        let explicit = path.is_some();
        let path = path.map(Path::to_path_buf).or_else(default_catalog_path);
        let Some(path) = path else {
            return Ok(catalog);
        };
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(source) if !explicit && source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(catalog);
            }
            Err(source) => {
                return Err(CatalogError::Read {
                    path: path.clone(),
                    source,
                });
            }
        };
        let document =
            toml::from_str::<CatalogDocument>(&source).map_err(|source| CatalogError::Parse {
                path: path.clone(),
                source,
            })?;

        if document.schema != CATALOG_SCHEMA {
            return Err(CatalogError::Schema {
                found: document.schema,
            });
        }

        let base = path.parent().unwrap_or_else(|| Path::new("."));
        let mut seen = BTreeMap::<String, &'static str>::new();

        for entry in document.entries {
            match entry {
                CatalogEntry::Template {
                    id,
                    path,
                    description,
                } => {
                    validate_id(&id)?;
                    insert_unique(&mut seen, &id, "template")?;
                    let path = if path.is_absolute() {
                        path
                    } else {
                        base.join(path)
                    };

                    catalog.templates.insert(
                        id,
                        TemplateEntry {
                            kind: TemplateKind::Application,
                            path: Some(path),
                            description,
                        },
                    );
                }
                CatalogEntry::Tool {
                    id,
                    command,
                    description,
                    package,
                    protocols,
                } => {
                    validate_id(&id)?;
                    insert_unique(&mut seen, &id, "tool")?;
                    catalog.tools.insert(
                        id.clone(),
                        ToolEntry {
                            id,
                            command,
                            description,
                            package,
                            protocols,
                        },
                    );
                }
            }
        }

        Ok(catalog)
    }

    /// Returns all effective template IDs in deterministic order.
    pub fn template_ids(&self) -> impl Iterator<Item = &str> {
        self.templates.keys().map(String::as_str)
    }

    /// Returns all configured developer tools in deterministic ID order.
    pub fn tools(&self) -> impl Iterator<Item = &ToolEntry> {
        self.tools.values()
    }

    /// Returns the selected template's human-readable summary.
    pub fn template_description(&self, id: &str) -> Result<&str, CatalogError> {
        Ok(&self.template(id)?.description)
    }

    pub(crate) fn template(&self, id: &str) -> Result<&TemplateEntry, CatalogError> {
        self.templates
            .get(id)
            .ok_or_else(|| CatalogError::UnknownTemplate {
                id: id.to_owned(),
                available: self.templates.keys().cloned().collect(),
            })
    }

    /// Returns the catalog compiled into this cargo-upwell release.
    pub fn builtins() -> Self {
        let templates = [
            (
                "upwell/application",
                TemplateKind::Application,
                "Named Upwell application crate",
            ),
            (
                "upwell/application-workspace",
                TemplateKind::ApplicationWorkspace,
                "Workspace containing a named Upwell application",
            ),
            (
                "upwell/plugin",
                TemplateKind::Plugin,
                "Protocol-neutral Upwell plugin crate",
            ),
            (
                "upwell/protocol",
                TemplateKind::Protocol,
                "First-class Upwell protocol crate",
            ),
        ]
        .into_iter()
        .map(|(id, kind, description)| {
            (
                id.to_owned(),
                TemplateEntry {
                    kind,
                    path: None,
                    description: description.to_owned(),
                },
            )
        })
        .collect();

        Self {
            templates,
            tools: BTreeMap::new(),
        }
    }
}

/// Platform-native path to the shared Upwell catalog file.
pub fn default_catalog_path() -> Option<PathBuf> {
    ProjectDirs::from("org", "upwell-rs", "Upwell")
        .map(|directories| directories.config_dir().join("catalog.toml"))
}

/// Catalog read, schema, or selection failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CatalogError {
    /// Catalog file could not be read.
    #[error("failed to read Upwell catalog `{path}`")]
    Read {
        /// Catalog path.
        path: PathBuf,
        /// Filesystem failure.
        #[source]
        source: std::io::Error,
    },
    /// Catalog TOML is invalid.
    #[error("failed to parse Upwell catalog `{path}`")]
    Parse {
        /// Catalog path.
        path: PathBuf,
        /// TOML failure.
        #[source]
        source: toml::de::Error,
    },
    /// Catalog schema is unsupported.
    #[error("unsupported Upwell catalog schema `{found}`; expected `{CATALOG_SCHEMA}`")]
    Schema {
        /// Unsupported schema value.
        found: String,
    },
    /// An entry ID is malformed.
    #[error("catalog entry ID `{0}` must be a non-empty namespaced ID")]
    InvalidId(String),
    /// One user catalog defines the same ID more than once.
    #[error("catalog entry `{id}` is declared more than once")]
    DuplicateId {
        /// Duplicated entry ID.
        id: String,
    },
    /// Requested template does not exist.
    #[error("unknown template `{id}`; available templates: {available}", available = available.join(", "))]
    UnknownTemplate {
        /// Requested ID.
        id: String,
        /// Deterministically ordered effective IDs.
        available: Vec<String>,
    },
}

fn validate_id(id: &str) -> Result<(), CatalogError> {
    let mut separators = 0;
    let valid = id.split('/').all(|segment| {
        separators += 1;
        segment
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_lowercase)
            && segment.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
            })
    });

    if valid && separators > 1 {
        Ok(())
    } else {
        Err(CatalogError::InvalidId(id.to_owned()))
    }
}

fn insert_unique(
    seen: &mut BTreeMap<String, &'static str>,
    id: &str,
    kind: &'static str,
) -> Result<(), CatalogError> {
    if seen.insert(id.to_owned(), kind).is_some() {
        Err(CatalogError::DuplicateId { id: id.to_owned() })
    } else {
        Ok(())
    }
}
