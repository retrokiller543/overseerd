use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::Deserialize;
use thiserror::Error;

const CATALOG_SCHEMA: &str = "1";
const TEMPLATE_TAG: &str = "v0.20.4";

/// Source of one cargo-generate template.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TemplateSource {
    /// A Git repository and optional immutable branch, tag, or revision selector.
    Git {
        /// Clone URL accepted by cargo-generate.
        repository: String,
        /// Optional Git reference.
        reference: Option<GitReference>,
    },
    /// A local template directory.
    Local(PathBuf),
}

/// Git selector applied to a remote template repository.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitReference {
    /// Branch name.
    Branch(String),
    /// Tag name.
    Tag(String),
    /// Exact commit or revision.
    Revision(String),
}

impl fmt::Display for GitReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Branch(branch) => write!(formatter, "branch {branch}"),
            Self::Tag(tag) => write!(formatter, "tag {tag}"),
            Self::Revision(revision) => write!(formatter, "revision {revision}"),
        }
    }
}

/// One effective template definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TemplateEntry {
    id: String,
    description: String,
    source: TemplateSource,
}

impl TemplateEntry {
    /// Stable namespaced catalog identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Human-readable template summary.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Git or local cargo-generate source.
    pub fn source(&self) -> &TemplateSource {
        &self.source
    }
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
    Template {
        id: String,
        description: String,
        #[serde(default)]
        git: Option<String>,
        #[serde(default)]
        path: Option<PathBuf>,
        #[serde(default)]
        branch: Option<String>,
        #[serde(default)]
        tag: Option<String>,
        #[serde(default, alias = "rev")]
        revision: Option<String>,
    },
    Tool {
        id: String,
        command: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        package: Option<String>,
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
                    description,
                    git,
                    path: template_path,
                    branch,
                    tag,
                    revision,
                } => {
                    validate_id(&id)?;
                    insert_unique(&mut seen, &id, "template")?;
                    let source =
                        resolve_source(base, &id, git, template_path, branch, tag, revision)?;

                    catalog.templates.insert(
                        id.clone(),
                        TemplateEntry {
                            id,
                            description,
                            source,
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

    /// Returns the catalog compiled into this cargo-upwell release.
    pub fn builtins() -> Self {
        let templates = [
            (
                "upwell/application",
                "Application or multi-crate workspace",
                "https://github.com/upwell-rs/upwell-template-application.git",
            ),
            (
                "upwell/plugin",
                "Reusable protocol-neutral plugin",
                "https://github.com/upwell-rs/upwell-template-plugin.git",
            ),
            (
                "upwell/protocol",
                "First-class protocol with an optional macro crate",
                "https://github.com/upwell-rs/upwell-template-protocol.git",
            ),
        ]
        .into_iter()
        .map(|(id, description, repository)| {
            (
                id.to_owned(),
                TemplateEntry {
                    id: id.to_owned(),
                    description: description.to_owned(),
                    source: TemplateSource::Git {
                        repository: repository.to_owned(),
                        reference: Some(GitReference::Tag(TEMPLATE_TAG.to_owned())),
                    },
                },
            )
        })
        .collect();

        Self {
            templates,
            tools: BTreeMap::new(),
        }
    }

    /// Returns all effective templates in deterministic ID order.
    pub fn templates(&self) -> impl Iterator<Item = &TemplateEntry> {
        self.templates.values()
    }

    /// Returns all configured developer tools in deterministic ID order.
    pub fn tools(&self) -> impl Iterator<Item = &ToolEntry> {
        self.tools.values()
    }

    pub(crate) fn template(&self, id: &str) -> Result<&TemplateEntry, CatalogError> {
        self.templates
            .get(id)
            .ok_or_else(|| CatalogError::UnknownTemplate {
                id: id.to_owned(),
                available: self.templates.keys().cloned().collect(),
            })
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
    #[error("failed to read Upwell catalog `{path}`")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse Upwell catalog `{path}`")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("unsupported Upwell catalog schema `{found}`; expected `{CATALOG_SCHEMA}`")]
    Schema { found: String },
    #[error("catalog entry ID `{0}` must be a valid namespaced ID")]
    InvalidId(String),
    #[error("catalog entry `{id}` is declared more than once")]
    DuplicateId { id: String },
    #[error("template `{id}` specifies more than one Git branch, tag, or revision")]
    ConflictingGitReferences { id: String },
    #[error("template `{id}` must specify exactly one of `git` or `path`")]
    InvalidTemplateSource { id: String },
    #[error("unknown template `{id}`; available templates: {available}", available = available.join(", "))]
    UnknownTemplate { id: String, available: Vec<String> },
}

fn resolve_source(
    base: &Path,
    id: &str,
    git: Option<String>,
    path: Option<PathBuf>,
    branch: Option<String>,
    tag: Option<String>,
    revision: Option<String>,
) -> Result<TemplateSource, CatalogError> {
    match (git, path) {
        (None, Some(path)) if branch.is_none() && tag.is_none() && revision.is_none() => {
            Ok(TemplateSource::Local(if path.is_absolute() {
                path
            } else {
                base.join(path)
            }))
        }
        (Some(repository), None) => {
            let references = [branch.is_some(), tag.is_some(), revision.is_some()]
                .into_iter()
                .filter(|present| *present)
                .count();
            if references > 1 {
                return Err(CatalogError::ConflictingGitReferences { id: id.to_owned() });
            }

            Ok(TemplateSource::Git {
                repository,
                reference: branch
                    .map(GitReference::Branch)
                    .or_else(|| tag.map(GitReference::Tag))
                    .or_else(|| revision.map(GitReference::Revision)),
            })
        }
        _ => Err(CatalogError::InvalidTemplateSource { id: id.to_owned() }),
    }
}

fn validate_id(id: &str) -> Result<(), CatalogError> {
    let mut segments = 0;
    let valid = id.split('/').all(|segment| {
        segments += 1;
        segment
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_lowercase)
            && segment.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
            })
    });

    if valid && segments > 1 {
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
