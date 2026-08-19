use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use semver::VersionReq;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::Catalog;

mod component;

pub use component::{
    ComponentLimits, ComponentRenderError, ComponentRenderRequest, ComponentRendererHost,
    run_component_compiler_worker,
};

/// Version requirement for the host/component renderer ABI.
pub const RENDERER_ABI_REQUIREMENT: &str = "^0.1";

/// A Cargo Upwell command that produces renderable output.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum RendererCommand {
    /// Application validation report.
    Check,
    /// Toolchain and application diagnostic report.
    Doctor,
    /// Prepared tooling-document inspection.
    Inspect,
    /// Canonical document or envelope export.
    Export,
    /// Derived resource graph.
    Graph,
    /// One derived resource explanation.
    Explain,
}

impl RendererCommand {
    /// Stable command-line spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Doctor => "doctor",
            Self::Inspect => "inspect",
            Self::Export => "export",
            Self::Graph => "graph",
            Self::Explain => "explain",
        }
    }
}

/// Output-policy capabilities declared by one renderer format.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RendererCapabilities {
    /// Whether ANSI color may be requested.
    pub color: bool,
    /// Whether output may pass through a pager.
    pub pager: bool,
    /// Whether output may be written through the atomic file-output path.
    pub output_file: bool,
}

/// One command and format claim made by a renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererFormat {
    command: RendererCommand,
    id: String,
    media_type: String,
    extension: Option<String>,
    capabilities: RendererCapabilities,
    default: bool,
}

impl RendererFormat {
    /// Command whose canonical projection this format consumes.
    pub const fn command(&self) -> RendererCommand {
        self.command
    }

    /// Stable CLI format ID.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// IANA media type emitted by this format.
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    /// Conventional file extension without a leading dot.
    pub fn extension(&self) -> Option<&str> {
        self.extension.as_deref()
    }

    /// Output-policy capabilities.
    pub const fn capabilities(&self) -> RendererCapabilities {
        self.capabilities
    }

    /// Whether this is the command's default format.
    pub const fn is_default(&self) -> bool {
        self.default
    }
}

/// Native implementation selected for a built-in renderer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BuiltInRenderer {
    ReportTerminal,
    ReportJson,
    InspectText,
    InspectJson,
    ExportDocument,
    ExportEnvelope,
    GraphText,
    GraphMermaid,
    GraphDot,
    GraphJson,
    ExplainText,
    ExplainJson,
}

/// Sandboxed component implementation and compatibility requirements.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentRenderer {
    path: PathBuf,
    abi: VersionReq,
    tooling_schema: VersionReq,
    utf8: bool,
}

impl ComponentRenderer {
    /// Explicit component file path.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Compatible renderer ABI versions.
    pub fn abi(&self) -> &VersionReq {
        &self.abi
    }

    /// Compatible canonical tooling-schema versions.
    pub fn tooling_schema(&self) -> &VersionReq {
        &self.tooling_schema
    }

    /// Whether the response body must be valid UTF-8.
    pub const fn utf8(&self) -> bool {
        self.utf8
    }
}

/// Executable implementation behind one renderer descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RendererImplementation {
    /// Statically linked Rust renderer.
    Native(BuiltInRenderer),
    /// Explicitly configured, capability-free WebAssembly component.
    Component(ComponentRenderer),
}

/// One renderer registration with one or more command/format claims.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererDescriptor {
    id: String,
    priority: i32,
    formats: Vec<RendererFormat>,
    implementation: RendererImplementation,
}

impl RendererDescriptor {
    /// Stable namespaced renderer identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Deterministic selection priority. Higher values win.
    pub const fn priority(&self) -> i32 {
        self.priority
    }

    /// Declared command/format claims.
    pub fn formats(&self) -> &[RendererFormat] {
        &self.formats
    }

    /// Native or component implementation.
    pub fn implementation(&self) -> &RendererImplementation {
        &self.implementation
    }
}

/// Deterministic built-in and user renderer registry.
#[derive(Clone, Debug)]
pub struct RendererRegistry {
    descriptors: BTreeMap<String, RendererDescriptor>,
    candidates: BTreeMap<(RendererCommand, String), Vec<String>>,
    defaults: BTreeMap<RendererCommand, String>,
}

impl RendererRegistry {
    /// Loads the shared user catalog and combines its components with native built-ins.
    pub fn load() -> Result<Self, RendererRegistryError> {
        Self::from_catalog(&Catalog::load(None)?)
    }

    /// Returns only the native renderers compiled into this release.
    pub fn builtins() -> Self {
        Self::new(built_in_descriptors()).expect("compiled renderer descriptors are valid")
    }

    /// Combines native built-ins with one already loaded shared catalog.
    pub fn from_catalog(catalog: &Catalog) -> Result<Self, RendererRegistryError> {
        let descriptors = built_in_descriptors()
            .into_iter()
            .chain(catalog.renderers().cloned())
            .collect::<Vec<_>>();

        Self::new(descriptors)
    }

    /// Builds a registry from registrations in any order.
    pub fn new(
        descriptors: impl IntoIterator<Item = RendererDescriptor>,
    ) -> Result<Self, RendererRegistryError> {
        let mut by_id = BTreeMap::new();

        for descriptor in descriptors {
            validate_descriptor(&descriptor)?;
            let id = descriptor.id.clone();

            if by_id.insert(id.clone(), descriptor).is_some() {
                return Err(RendererRegistryError::DuplicateRenderer { id });
            }
        }

        let mut candidates = BTreeMap::<(RendererCommand, String), Vec<&RendererDescriptor>>::new();
        let mut defaults = BTreeMap::new();

        for descriptor in by_id.values() {
            for format in &descriptor.formats {
                let key = (format.command, format.id.clone());
                candidates.entry(key).or_default().push(descriptor);

                if format.default && defaults.insert(format.command, format.id.clone()).is_some() {
                    return Err(RendererRegistryError::DuplicateDefault {
                        command: format.command,
                    });
                }
            }
        }

        for command in commands() {
            if !defaults.contains_key(&command) {
                return Err(RendererRegistryError::MissingDefault { command });
            }
        }

        let candidates = candidates
            .into_iter()
            .map(|(key, mut candidates)| {
                if candidates.len() > 1
                    && candidates.iter().any(|candidate| {
                        matches!(candidate.implementation, RendererImplementation::Native(_))
                    })
                {
                    let component = candidates
                        .iter()
                        .find(|candidate| {
                            matches!(
                                candidate.implementation,
                                RendererImplementation::Component(_)
                            )
                        })
                        .expect("multiple native/component candidates include a component");

                    return Err(RendererRegistryError::ReservedBuiltInFormat {
                        id: component.id.clone(),
                        command: key.0,
                        format: key.1,
                    });
                }
                candidates.sort_by(|left, right| {
                    right
                        .priority
                        .cmp(&left.priority)
                        .then_with(|| left.id.cmp(&right.id))
                });

                Ok((
                    key,
                    candidates
                        .into_iter()
                        .map(|candidate| candidate.id.clone())
                        .collect(),
                ))
            })
            .collect::<Result<_, _>>()?;

        Ok(Self {
            descriptors: by_id,
            candidates,
            defaults,
        })
    }

    /// Resolves one command-scoped format deterministically.
    pub fn resolve(&self, command: RendererCommand, format: &str) -> Option<ResolvedRenderer<'_>> {
        let renderer = self
            .candidates
            .get(&(command, format.to_owned()))?
            .first()?;
        let descriptor = &self.descriptors[renderer];
        let format = descriptor
            .formats
            .iter()
            .find(|claim| claim.command == command && claim.id == format)?;

        Some(ResolvedRenderer { descriptor, format })
    }

    /// Resolves the native fallback for a command and format, when one exists.
    pub fn native_fallback(
        &self,
        command: RendererCommand,
        format: &str,
    ) -> Option<ResolvedRenderer<'_>> {
        let renderers = self.candidates.get(&(command, format.to_owned()))?;
        let descriptor = renderers.iter().find_map(|renderer| {
            let descriptor = &self.descriptors[renderer];

            matches!(descriptor.implementation, RendererImplementation::Native(_))
                .then_some(descriptor)
        })?;
        let format = descriptor
            .formats
            .iter()
            .find(|claim| claim.command == command && claim.id == format)?;

        Some(ResolvedRenderer { descriptor, format })
    }

    /// Resolves the command's native default renderer for component failure fallback.
    pub fn command_fallback(&self, command: RendererCommand) -> ResolvedRenderer<'_> {
        let format = self.default_format(command);

        self.native_fallback(command, format)
            .expect("every compiled command default is native")
    }

    /// Returns the command's built-in default format ID.
    pub fn default_format(&self, command: RendererCommand) -> &str {
        &self.defaults[&command]
    }

    /// Returns effective formats in stable ID order for help and completion.
    pub fn formats(&self, command: RendererCommand) -> impl Iterator<Item = ResolvedRenderer<'_>> {
        self.candidates
            .iter()
            .filter(move |((candidate, _), _)| *candidate == command)
            .filter_map(move |((_, format), _)| self.resolve(command, format))
    }
}

/// One resolved command/format implementation.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedRenderer<'a> {
    descriptor: &'a RendererDescriptor,
    format: &'a RendererFormat,
}

impl<'a> ResolvedRenderer<'a> {
    pub fn descriptor(self) -> &'a RendererDescriptor {
        self.descriptor
    }

    pub fn format(self) -> &'a RendererFormat {
        self.format
    }
}

/// Renderer registration or selection failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RendererRegistryError {
    #[error(transparent)]
    Catalog(#[from] crate::CatalogError),
    #[error("renderer `{id}` is registered more than once")]
    DuplicateRenderer { id: String },
    #[error("renderer `{id}` declares `{command}` format `{format}` more than once")]
    DuplicateFormat {
        id: String,
        command: RendererCommand,
        format: String,
    },
    #[error("more than one default renderer format is registered for `{command}`")]
    DuplicateDefault { command: RendererCommand },
    #[error("no default renderer format is registered for `{command}`")]
    MissingDefault { command: RendererCommand },
    #[error("renderer `{id}` has no command/format claims")]
    EmptyRenderer { id: String },
    #[error("renderer `{id}` has an invalid empty {field}")]
    EmptyValue { id: String, field: &'static str },
    #[error("renderer `{id}` cannot enable {capability} for non-text media type `{media_type}`")]
    MachinePresentationCapability {
        id: String,
        capability: &'static str,
        media_type: String,
    },
    #[error("renderer `{id}` declares unsupported {capability} capability for `{command}`")]
    UnsupportedCommandCapability {
        id: String,
        command: RendererCommand,
        capability: &'static str,
    },
    #[error("renderer `{id}` has invalid format ID `{format}`")]
    InvalidFormatId { id: String, format: String },
    #[error("renderer `{id}` has invalid media type `{media_type}`")]
    InvalidMediaType { id: String, media_type: String },
    #[error("renderer `{id}` has invalid file extension `{extension}`")]
    InvalidExtension { id: String, extension: String },
    #[error("renderer `{id}` cannot enable color when UTF-8 output validation is disabled")]
    BinaryColorOutput { id: String },
    #[error("renderer `{id}` cannot emit binary output for `{command}`")]
    BinaryCommandOutput {
        id: String,
        command: RendererCommand,
    },
    #[error("renderer `{id}` cannot replace reserved built-in `{command}` format `{format}`")]
    ReservedBuiltInFormat {
        id: String,
        command: RendererCommand,
        format: String,
    },
}

impl std::fmt::Display for RendererCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

fn validate_descriptor(descriptor: &RendererDescriptor) -> Result<(), RendererRegistryError> {
    if descriptor.formats.is_empty() {
        return Err(RendererRegistryError::EmptyRenderer {
            id: descriptor.id.clone(),
        });
    }

    let mut claims = BTreeSet::new();
    for format in &descriptor.formats {
        for (field, value) in [
            ("format ID", format.id.as_str()),
            ("media type", format.media_type.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(RendererRegistryError::EmptyValue {
                    id: descriptor.id.clone(),
                    field,
                });
            }
        }
        if !valid_token(&format.id) {
            return Err(RendererRegistryError::InvalidFormatId {
                id: descriptor.id.clone(),
                format: format.id.clone(),
            });
        }
        if !valid_media_type(&format.media_type) {
            return Err(RendererRegistryError::InvalidMediaType {
                id: descriptor.id.clone(),
                media_type: format.media_type.clone(),
            });
        }
        if format
            .extension
            .as_deref()
            .is_some_and(|extension| !valid_token(extension))
        {
            return Err(RendererRegistryError::InvalidExtension {
                id: descriptor.id.clone(),
                extension: format.extension.clone().unwrap_or_default(),
            });
        }

        if !claims.insert((format.command, format.id.clone())) {
            return Err(RendererRegistryError::DuplicateFormat {
                id: descriptor.id.clone(),
                command: format.command,
                format: format.id.clone(),
            });
        }

        if !format.media_type.starts_with("text/") {
            for (capability, enabled) in [
                ("color", format.capabilities.color),
                ("paging", format.capabilities.pager),
            ] {
                if enabled {
                    return Err(RendererRegistryError::MachinePresentationCapability {
                        id: descriptor.id.clone(),
                        capability,
                        media_type: format.media_type.clone(),
                    });
                }
            }
        }
        if format.capabilities.color
            && matches!(descriptor.implementation, RendererImplementation::Component(ref component) if !component.utf8)
        {
            return Err(RendererRegistryError::BinaryColorOutput {
                id: descriptor.id.clone(),
            });
        }
        if format.command != RendererCommand::Export
            && matches!(descriptor.implementation, RendererImplementation::Component(ref component) if !component.utf8)
        {
            return Err(RendererRegistryError::BinaryCommandOutput {
                id: descriptor.id.clone(),
                command: format.command,
            });
        }
        let terminal_options = matches!(
            format.command,
            RendererCommand::Inspect | RendererCommand::Graph | RendererCommand::Explain
        );
        for (capability, enabled, supported) in [
            ("color", format.capabilities.color, terminal_options),
            ("paging", format.capabilities.pager, terminal_options),
            (
                "output-file",
                format.capabilities.output_file,
                format.command == RendererCommand::Export,
            ),
        ] {
            if enabled && !supported {
                return Err(RendererRegistryError::UnsupportedCommandCapability {
                    id: descriptor.id.clone(),
                    command: format.command,
                    capability,
                });
            }
        }
    }

    Ok(())
}

fn valid_token(value: &str) -> bool {
    value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
}

fn valid_media_type(value: &str) -> bool {
    let Some((category, subtype)) = value.split_once('/') else {
        return false;
    };

    valid_token(category)
        && subtype
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_lowercase)
        && subtype.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._+-".contains(&byte)
        })
}

fn commands() -> [RendererCommand; 6] {
    [
        RendererCommand::Check,
        RendererCommand::Doctor,
        RendererCommand::Inspect,
        RendererCommand::Export,
        RendererCommand::Graph,
        RendererCommand::Explain,
    ]
}

fn built_in_descriptors() -> Vec<RendererDescriptor> {
    use BuiltInRenderer as Native;
    use RendererCommand as Command;

    vec![
        native(
            "upwell/report-terminal",
            Native::ReportTerminal,
            &[
                (Command::Check, "terminal", true),
                (Command::Doctor, "terminal", true),
            ],
            "text/plain",
            Some("txt"),
            RendererCapabilities {
                color: false,
                pager: false,
                output_file: false,
            },
        ),
        native(
            "upwell/report-json",
            Native::ReportJson,
            &[
                (Command::Check, "json", false),
                (Command::Doctor, "json", false),
            ],
            "application/json",
            Some("json"),
            RendererCapabilities::default(),
        ),
        native(
            "upwell/inspect-text",
            Native::InspectText,
            &[(Command::Inspect, "text", true)],
            "text/plain",
            Some("txt"),
            RendererCapabilities {
                color: true,
                pager: true,
                output_file: false,
            },
        ),
        native(
            "upwell/inspect-json",
            Native::InspectJson,
            &[(Command::Inspect, "json", false)],
            "application/json",
            Some("json"),
            RendererCapabilities::default(),
        ),
        native(
            "upwell/export-document",
            Native::ExportDocument,
            &[(Command::Export, "document", true)],
            "application/json",
            Some("json"),
            RendererCapabilities {
                color: false,
                pager: false,
                output_file: true,
            },
        ),
        native(
            "upwell/export-envelope",
            Native::ExportEnvelope,
            &[(Command::Export, "envelope", false)],
            "application/json",
            Some("json"),
            RendererCapabilities {
                color: false,
                pager: false,
                output_file: true,
            },
        ),
        native(
            "upwell/graph-text",
            Native::GraphText,
            &[(Command::Graph, "text", true)],
            "text/plain",
            Some("txt"),
            RendererCapabilities {
                color: true,
                pager: true,
                output_file: false,
            },
        ),
        native(
            "upwell/graph-mermaid",
            Native::GraphMermaid,
            &[(Command::Graph, "mermaid", false)],
            "text/vnd.mermaid",
            Some("mmd"),
            RendererCapabilities::default(),
        ),
        native(
            "upwell/graph-dot",
            Native::GraphDot,
            &[(Command::Graph, "dot", false)],
            "text/vnd.graphviz",
            Some("dot"),
            RendererCapabilities::default(),
        ),
        native(
            "upwell/graph-json",
            Native::GraphJson,
            &[(Command::Graph, "json", false)],
            "application/json",
            Some("json"),
            RendererCapabilities::default(),
        ),
        native(
            "upwell/explain-text",
            Native::ExplainText,
            &[(Command::Explain, "text", true)],
            "text/plain",
            Some("txt"),
            RendererCapabilities {
                color: true,
                pager: true,
                output_file: false,
            },
        ),
        native(
            "upwell/explain-json",
            Native::ExplainJson,
            &[(Command::Explain, "json", false)],
            "application/json",
            Some("json"),
            RendererCapabilities::default(),
        ),
    ]
}

fn native(
    id: &str,
    implementation: BuiltInRenderer,
    claims: &[(RendererCommand, &str, bool)],
    media_type: &str,
    extension: Option<&str>,
    capabilities: RendererCapabilities,
) -> RendererDescriptor {
    RendererDescriptor {
        id: id.to_owned(),
        priority: 0,
        formats: claims
            .iter()
            .map(|(command, id, default)| RendererFormat {
                command: *command,
                id: (*id).to_owned(),
                media_type: media_type.to_owned(),
                extension: extension.map(str::to_owned),
                capabilities,
                default: *default,
            })
            .collect(),
        implementation: RendererImplementation::Native(implementation),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn component_descriptor(
    id: String,
    path: PathBuf,
    commands: Vec<RendererCommand>,
    format: String,
    media_type: String,
    extension: Option<String>,
    capabilities: RendererCapabilities,
    priority: i32,
    abi: VersionReq,
    tooling_schema: VersionReq,
    utf8: bool,
) -> RendererDescriptor {
    RendererDescriptor {
        id,
        priority,
        formats: commands
            .into_iter()
            .map(|command| RendererFormat {
                command,
                id: format.clone(),
                media_type: media_type.clone(),
                extension: extension.clone(),
                capabilities,
                default: false,
            })
            .collect(),
        implementation: RendererImplementation::Component(ComponentRenderer {
            path,
            abi,
            tooling_schema,
            utf8,
        }),
    }
}

#[cfg(test)]
mod tests;
