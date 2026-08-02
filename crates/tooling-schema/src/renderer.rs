//! Versioned presentation-only contract for optional tooling renderers.

use std::collections::{BTreeMap, BTreeSet};

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{ResourceKind, TOOLING_SCHEMA_VERSION, ToolingDocument, ValidationError};

/// Hidden process argument selecting the display-renderer contract.
pub const TOOLING_RENDERER_ARGUMENT: &str = "--__overseerd-tooling-renderer-v1";

/// Environment variable containing the renderer request file.
pub const TOOLING_RENDERER_REQUEST_ENV: &str = "OVERSEERD_TOOLING_RENDERER_REQUEST";

/// Environment variable containing the renderer response file.
pub const TOOLING_RENDERER_RESPONSE_ENV: &str = "OVERSEERD_TOOLING_RENDERER_RESPONSE";

/// Tooling command view for which presentation hints are requested.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum RendererView {
    /// Human-readable application inspection.
    Inspect,
    /// Human-readable resource graph.
    Graph,
    /// Human-readable explanation of one resource.
    Explain,
}

/// Explicit local configuration authorizing one renderer executable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RendererManifest {
    /// Renderer contract compatibility version.
    pub schema: Version,
    /// Stable renderer identity used in diagnostics.
    pub id: String,
    /// Exact protocol or plugin resource identity owned by this renderer.
    pub owner: String,
    /// Executable path, relative to this manifest when not absolute.
    pub executable: String,
    /// Accepted tooling-document versions.
    pub document_schema: VersionReq,
    /// Exact owner-qualified facet versions understood by the renderer.
    pub facets: BTreeMap<String, Vec<u16>>,
}

impl RendererManifest {
    /// Validates renderer identity, compatibility, and owner-local facet claims.
    pub fn validate(&self) -> Result<(), RendererValidationError> {
        validate_contract_schema(&self.schema)?;

        if self.id.trim().is_empty() {
            return Err(RendererValidationError::MissingRendererId);
        }

        validate_owner(&self.owner)?;

        if self.executable.trim().is_empty() {
            return Err(RendererValidationError::MissingExecutable);
        }

        if self.facets.is_empty() {
            return Err(RendererValidationError::MissingFacetClaim);
        }

        let prefix = format!("{}/tooling/", self.owner);

        for (facet, versions) in &self.facets {
            if !facet.starts_with(&prefix) {
                return Err(RendererValidationError::ForeignFacet {
                    owner: self.owner.clone(),
                    facet: facet.clone(),
                });
            }

            if versions.is_empty() || versions.contains(&0) {
                return Err(RendererValidationError::InvalidFacetVersions {
                    facet: facet.clone(),
                });
            }
        }

        Ok(())
    }

    /// Validates this renderer against one immutable tooling document.
    pub fn validate_document(
        &self,
        document: &ToolingDocument,
    ) -> Result<(), RendererValidationError> {
        self.validate()?;
        document.validate()?;

        if !self.document_schema.matches(&document.schema) {
            return Err(RendererValidationError::IncompatibleDocument {
                actual: document.schema.clone(),
                required: self.document_schema.clone(),
            });
        }

        let owner = document
            .resources
            .iter()
            .find(|resource| resource.id == self.owner)
            .ok_or_else(|| RendererValidationError::UnknownOwner {
                owner: self.owner.clone(),
            })?;

        if !matches!(owner.kind, ResourceKind::Protocol | ResourceKind::Plugin) {
            return Err(RendererValidationError::InvalidOwnerKind {
                owner: self.owner.clone(),
            });
        }

        for (facet_id, supported) in &self.facets {
            let facet = document
                .resources
                .iter()
                .flat_map(|resource| resource.facets.iter())
                .chain(document.facets.iter())
                .find_map(|(id, facet)| (id == facet_id).then_some(facet))
                .ok_or_else(|| RendererValidationError::MissingFacet {
                    facet: facet_id.clone(),
                })?;

            if !supported.contains(&facet.schema_version) {
                return Err(RendererValidationError::UnsupportedFacetVersion {
                    facet: facet_id.clone(),
                    actual: facet.schema_version,
                    supported: supported.clone(),
                });
            }
        }

        Ok(())
    }
}

/// Immutable input supplied to one authorized display renderer.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RendererRequest {
    /// Renderer contract compatibility version.
    pub schema: Version,
    /// Stable renderer identity from the manifest.
    pub renderer: String,
    /// Exact owner identity from the manifest.
    pub owner: String,
    /// Text view being presented.
    pub view: RendererView,
    /// Validated immutable generic tooling document.
    pub document: ToolingDocument,
    /// Resource IDs selected for this view.
    pub resources: Vec<String>,
}

impl RendererRequest {
    /// Creates and validates one renderer request.
    pub fn new(
        manifest: &RendererManifest,
        document: &ToolingDocument,
        view: RendererView,
        resources: impl IntoIterator<Item = String>,
    ) -> Result<Self, RendererValidationError> {
        manifest.validate_document(document)?;

        let mut request = Self {
            schema: TOOLING_SCHEMA_VERSION,
            renderer: manifest.id.clone(),
            owner: manifest.owner.clone(),
            view,
            document: document.clone(),
            resources: resources.into_iter().collect(),
        };

        request.resources.sort();
        request.resources.dedup();
        request.validate()?;

        Ok(request)
    }

    /// Validates the request and every selected resource reference.
    pub fn validate(&self) -> Result<(), RendererValidationError> {
        validate_contract_schema(&self.schema)?;
        validate_owner(&self.owner)?;
        self.document.validate()?;

        if self.renderer.trim().is_empty() {
            return Err(RendererValidationError::MissingRendererId);
        }

        let known = self
            .document
            .resources
            .iter()
            .map(|resource| resource.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut selected = BTreeSet::new();

        for resource in &self.resources {
            if !known.contains(resource.as_str()) {
                return Err(RendererValidationError::UnknownResource {
                    resource: resource.clone(),
                });
            }

            if !selected.insert(resource) {
                return Err(RendererValidationError::DuplicateResource {
                    resource: resource.clone(),
                });
            }
        }

        Ok(())
    }

    /// Emits compact JSON after structural validation.
    pub fn to_json(&self) -> Result<String, RendererEmitError> {
        let mut request = self.clone();

        request.document.canonicalize();
        request.resources.sort();
        request.resources.dedup();
        request.validate()?;

        Ok(serde_json::to_string(&request)?)
    }

    /// Decodes and validates one renderer request.
    pub fn from_json(json: &str) -> Result<Self, RendererDecodeError> {
        let request: Self = serde_json::from_str(json)?;

        request.validate()?;

        Ok(request)
    }
}

/// Presentation hints for one generic resource.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourcePresentation {
    /// Exact resource identity from the request document.
    pub resource: String,
    /// Optional compact display label.
    #[serde(default)]
    pub label: Option<String>,
    /// Optional owner-defined display group.
    #[serde(default)]
    pub group: Option<String>,
    /// Optional one-line summary.
    #[serde(default)]
    pub summary: Option<String>,
    /// Deterministically keyed detail fields.
    #[serde(default)]
    pub details: BTreeMap<String, String>,
}

/// Validated discardable presentation overlay returned by renderers.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RendererPresentation {
    /// Owner-scoped resource hints in stable resource order.
    #[serde(default)]
    pub resources: Vec<ResourcePresentation>,
}

impl RendererPresentation {
    /// Returns presentation hints for one exact resource identity.
    pub fn resource(&self, id: &str) -> Option<&ResourcePresentation> {
        self.resources
            .iter()
            .find(|resource| resource.resource == id)
    }

    /// Merges one independently validated renderer overlay.
    pub fn merge(&mut self, mut other: Self) -> Result<(), RendererValidationError> {
        let mut identities = self
            .resources
            .iter()
            .map(|resource| resource.resource.as_str())
            .collect::<BTreeSet<_>>();

        if let Some(duplicate) = other
            .resources
            .iter()
            .find(|resource| !identities.insert(resource.resource.as_str()))
        {
            return Err(RendererValidationError::DuplicatePresentation {
                resource: duplicate.resource.clone(),
            });
        }

        self.resources.append(&mut other.resources);
        self.resources
            .sort_by(|left, right| left.resource.cmp(&right.resource));

        Ok(())
    }
}

/// Versioned renderer response containing presentation only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RendererResponse {
    /// Renderer contract compatibility version.
    pub schema: Version,
    /// Stable renderer identity echoed from the request.
    pub renderer: String,
    /// Exact owner identity echoed from the request.
    pub owner: String,
    /// Discardable owner-scoped presentation hints.
    pub presentation: RendererPresentation,
}

impl RendererResponse {
    /// Validates response identity and every presentation hint against its request.
    pub fn validate(&self, request: &RendererRequest) -> Result<(), RendererValidationError> {
        validate_contract_schema(&self.schema)?;

        if self.renderer != request.renderer || self.owner != request.owner {
            return Err(RendererValidationError::ResponseIdentityMismatch);
        }

        let mut identities = BTreeSet::new();

        for presentation in &self.presentation.resources {
            if !identities.insert(presentation.resource.as_str()) {
                return Err(RendererValidationError::DuplicatePresentation {
                    resource: presentation.resource.clone(),
                });
            }

            let resource = request
                .document
                .resources
                .iter()
                .find(|resource| resource.id == presentation.resource)
                .ok_or_else(|| RendererValidationError::UnknownResource {
                    resource: presentation.resource.clone(),
                })?;

            if !request
                .resources
                .iter()
                .any(|selected| selected == &presentation.resource)
            {
                return Err(RendererValidationError::UnselectedResource {
                    resource: presentation.resource.clone(),
                });
            }

            if !resource_is_owned(resource, &request.owner) {
                return Err(RendererValidationError::ForeignResource {
                    owner: request.owner.clone(),
                    resource: presentation.resource.clone(),
                });
            }

            validate_presentation(presentation)?;
        }

        Ok(())
    }

    /// Decodes and validates one renderer response.
    pub fn from_json(json: &str, request: &RendererRequest) -> Result<Self, RendererDecodeError> {
        let mut response: Self = serde_json::from_str(json)?;

        response
            .presentation
            .resources
            .sort_by(|left, right| left.resource.cmp(&right.resource));
        response.validate(request)?;

        Ok(response)
    }

    /// Emits compact JSON after validating against the originating request.
    pub fn to_json(&self, request: &RendererRequest) -> Result<String, RendererEmitError> {
        let mut response = self.clone();

        response
            .presentation
            .resources
            .sort_by(|left, right| left.resource.cmp(&right.resource));
        response.validate(request)?;

        Ok(serde_json::to_string(&response)?)
    }
}

/// A structural renderer contract violation.
#[derive(Clone, Debug, Error, PartialEq)]
#[non_exhaustive]
pub enum RendererValidationError {
    /// Renderer contract version is incompatible with this package.
    #[error("incompatible renderer contract schema {actual}; expected {required}")]
    IncompatibleSchema {
        /// Encountered contract version.
        actual: Version,
        /// Compatible contract releases.
        required: VersionReq,
    },
    /// Stable renderer identity is absent.
    #[error("renderer manifest has no stable identity")]
    MissingRendererId,
    /// Owner is not an exact protocol or plugin identity.
    #[error("renderer owner '{owner}' is not a protocol or plugin identity")]
    InvalidOwner {
        /// Invalid owner identity.
        owner: String,
    },
    /// Renderer executable path is absent.
    #[error("renderer manifest has no executable path")]
    MissingExecutable,
    /// Renderer declares no facet compatibility.
    #[error("renderer manifest declares no owner facet")]
    MissingFacetClaim,
    /// Facet claim is outside the renderer owner namespace.
    #[error("renderer for '{owner}' cannot claim facet '{facet}'")]
    ForeignFacet {
        /// Renderer owner.
        owner: String,
        /// Invalid facet identity.
        facet: String,
    },
    /// Facet compatibility list is empty or contains version zero.
    #[error("renderer facet '{facet}' has invalid supported versions")]
    InvalidFacetVersions {
        /// Invalid facet identity.
        facet: String,
    },
    /// Tooling document violates the generic schema.
    #[error(transparent)]
    Document(#[from] ValidationError),
    /// Tooling-document schema is not accepted by the renderer.
    #[error("renderer does not support tooling schema {actual}; expected {required}")]
    IncompatibleDocument {
        /// Encountered document version.
        actual: Version,
        /// Renderer requirement.
        required: VersionReq,
    },
    /// Renderer owner is absent from the document.
    #[error("renderer owner '{owner}' is absent from the tooling document")]
    UnknownOwner {
        /// Missing owner identity.
        owner: String,
    },
    /// Renderer owner exists but is not a protocol or plugin resource.
    #[error("renderer owner '{owner}' is not a protocol or plugin resource")]
    InvalidOwnerKind {
        /// Invalid owner resource.
        owner: String,
    },
    /// A claimed facet is absent from the document.
    #[error("renderer facet '{facet}' is absent from the tooling document")]
    MissingFacet {
        /// Missing facet identity.
        facet: String,
    },
    /// A claimed facet uses an unsupported version.
    #[error(
        "renderer facet '{facet}' uses unsupported version {actual}; supported versions are {supported:?}"
    )]
    UnsupportedFacetVersion {
        /// Facet identity.
        facet: String,
        /// Encountered version.
        actual: u16,
        /// Exact supported versions.
        supported: Vec<u16>,
    },
    /// A request or response references an unknown resource.
    #[error("renderer references unknown resource '{resource}'")]
    UnknownResource {
        /// Unknown resource identity.
        resource: String,
    },
    /// A response references a known resource outside the request selection.
    #[error("renderer response references unselected resource '{resource}'")]
    UnselectedResource {
        /// Unselected resource identity.
        resource: String,
    },
    /// A request repeats a selected resource.
    #[error("renderer request repeats resource '{resource}'")]
    DuplicateResource {
        /// Repeated resource identity.
        resource: String,
    },
    /// Response renderer or owner does not match the request.
    #[error("renderer response identity does not match its request")]
    ResponseIdentityMismatch,
    /// A response repeats presentation hints for one resource.
    #[error("renderer repeats presentation for resource '{resource}'")]
    DuplicatePresentation {
        /// Repeated resource identity.
        resource: String,
    },
    /// A response attempts to present another owner's resource.
    #[error("renderer for '{owner}' cannot present foreign resource '{resource}'")]
    ForeignResource {
        /// Renderer owner.
        owner: String,
        /// Foreign resource identity.
        resource: String,
    },
    /// A presentation entry contains no hint.
    #[error("renderer presentation for '{resource}' is empty")]
    EmptyPresentation {
        /// Resource identity.
        resource: String,
    },
    /// A supplied presentation field is blank and could suppress generic fallback text.
    #[error("renderer presentation for '{resource}' contains blank text")]
    BlankPresentationText {
        /// Resource identity.
        resource: String,
    },
    /// Renderer-provided presentation text exceeds the contract bound.
    #[error("renderer presentation text for '{resource}' exceeds the supported size")]
    PresentationTooLarge {
        /// Resource identity.
        resource: String,
    },
}

/// Failure while decoding a renderer request or response.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RendererDecodeError {
    /// JSON could not be decoded.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Decoded data violates the renderer contract.
    #[error(transparent)]
    Validation(#[from] RendererValidationError),
}

/// Failure while emitting a renderer request or response.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RendererEmitError {
    /// Contract data violates structural invariants.
    #[error(transparent)]
    Validation(#[from] RendererValidationError),
    /// JSON could not be emitted.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn validate_contract_schema(schema: &Version) -> Result<(), RendererValidationError> {
    let required = VersionReq::parse(&format!(
        "^{}.{}",
        TOOLING_SCHEMA_VERSION.major, TOOLING_SCHEMA_VERSION.minor
    ))
    .expect("the package-derived renderer contract requirement is valid");

    if !required.matches(schema) {
        return Err(RendererValidationError::IncompatibleSchema {
            actual: schema.clone(),
            required,
        });
    }

    Ok(())
}

fn validate_owner(owner: &str) -> Result<(), RendererValidationError> {
    let valid = ["protocol:", "plugin:"].iter().any(|prefix| {
        owner
            .strip_prefix(prefix)
            .is_some_and(|identity| identity.contains('/') && !identity.ends_with('/'))
    });

    if !valid {
        return Err(RendererValidationError::InvalidOwner {
            owner: owner.to_string(),
        });
    }

    Ok(())
}

fn resource_is_owned(resource: &crate::Resource, owner: &str) -> bool {
    if resource.id == owner {
        return true;
    }

    match resource
        .provenance
        .as_ref()
        .and_then(|provenance| provenance.owner.as_deref())
    {
        Some(provenance_owner) => provenance_owner == owner,
        None => resource.id.starts_with(&format!("{owner}/tooling/")),
    }
}

fn validate_presentation(
    presentation: &ResourcePresentation,
) -> Result<(), RendererValidationError> {
    const MAX_TEXT_BYTES: usize = 16 * 1024;

    let optional_values = presentation
        .label
        .iter()
        .chain(&presentation.group)
        .chain(&presentation.summary);

    if optional_values.clone().any(|value| value.trim().is_empty())
        || presentation
            .details
            .iter()
            .any(|(name, value)| name.trim().is_empty() || value.trim().is_empty())
    {
        return Err(RendererValidationError::BlankPresentationText {
            resource: presentation.resource.clone(),
        });
    }

    let values = optional_values
        .chain(presentation.details.keys())
        .chain(presentation.details.values());
    let mut values = values.peekable();
    let has_value = values.peek().is_some();
    let oversized = values
        .map(String::len)
        .any(|length| length > MAX_TEXT_BYTES);

    if !has_value {
        return Err(RendererValidationError::EmptyPresentation {
            resource: presentation.resource.clone(),
        });
    }

    if oversized {
        return Err(RendererValidationError::PresentationTooLarge {
            resource: presentation.resource.clone(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests;
