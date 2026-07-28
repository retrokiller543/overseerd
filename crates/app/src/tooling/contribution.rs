use std::collections::{BTreeMap, BTreeSet};

use overseerd_tooling_schema::{
    Facet, Provenance, Relationship, RelationshipKind, Resource, ResourceKind,
};

/// One endpoint in an owner-scoped tooling relationship.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolingEndpoint<'a> {
    /// The protocol or plugin that owns this collector.
    Owner,
    /// A resource declared in this collector, addressed by its owner-local identity.
    Resource(&'a str),
}

/// Relationship semantics available to protocol and plugin tooling extensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolingRelationshipKind {
    /// The source contains the target.
    Contains,
    /// The source depends on the target.
    DependsOn,
    /// The source must precede the target.
    OrdersBefore,
    /// The source must follow the target.
    OrdersAfter,
}

impl From<ToolingRelationshipKind> for RelationshipKind {
    fn from(kind: ToolingRelationshipKind) -> Self {
        match kind {
            ToolingRelationshipKind::Contains => Self::Contains,
            ToolingRelationshipKind::DependsOn => Self::DependsOn,
            ToolingRelationshipKind::OrdersBefore => Self::OrdersBefore,
            ToolingRelationshipKind::OrdersAfter => Self::OrdersAfter,
        }
    }
}

/// Generic, protocol-neutral metadata contributed by one prepared protocol or plugin.
///
/// Resource and facet identities supplied to this collector are owner-local. The framework
/// qualifies them with the protocol or plugin identity when preparation is projected, preventing
/// an extension from claiming framework or another contributor's resources.
#[derive(Debug)]
pub struct ToolingContributions {
    owner: String,
    resources: Vec<PendingResource>,
    relationships: Vec<PendingRelationship>,
    facets: Vec<PendingFacet>,
}

impl ToolingContributions {
    pub(crate) fn new(owner: String) -> Self {
        Self {
            owner,
            resources: Vec::new(),
            relationships: Vec::new(),
            facets: Vec::new(),
        }
    }

    /// Declares an owner-local contribution resource without scalar labels.
    pub fn resource(&mut self, id: impl Into<String>, name: impl Into<String>) {
        self.resource_with_labels(id, name, BTreeMap::new());
    }

    /// Declares an owner-local contribution resource with deterministic scalar labels.
    pub fn resource_with_labels(
        &mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        labels: BTreeMap<String, String>,
    ) {
        self.resources.push(PendingResource {
            id: id.into(),
            name: name.into(),
            labels,
        });
    }

    /// Declares a relationship whose endpoints are this owner or owner-local resources.
    pub fn relationship(
        &mut self,
        kind: ToolingRelationshipKind,
        from: ToolingEndpoint<'_>,
        to: ToolingEndpoint<'_>,
    ) {
        self.relationship_with_labels(kind, from, to, BTreeMap::new());
    }

    /// Declares a relationship with deterministic scalar labels.
    pub fn relationship_with_labels(
        &mut self,
        kind: ToolingRelationshipKind,
        from: ToolingEndpoint<'_>,
        to: ToolingEndpoint<'_>,
        labels: BTreeMap<String, String>,
    ) {
        self.relationships.push(PendingRelationship {
            kind: kind.into(),
            from: PendingEndpoint::from(from),
            to: PendingEndpoint::from(to),
            labels,
        });
    }

    /// Attaches an owner-local opaque facet to the protocol or plugin itself.
    pub fn facet(
        &mut self,
        id: impl Into<String>,
        schema_version: u16,
        value: overseerd_tooling_schema::JsonValue,
    ) {
        self.facets.push(PendingFacet {
            resource: None,
            id: id.into(),
            facet: Facet {
                schema_version,
                value,
            },
        });
    }

    /// Attaches an owner-local opaque facet to a resource in this collector.
    pub fn resource_facet(
        &mut self,
        resource: impl Into<String>,
        id: impl Into<String>,
        schema_version: u16,
        value: overseerd_tooling_schema::JsonValue,
    ) {
        self.facets.push(PendingFacet {
            resource: Some(resource.into()),
            id: id.into(),
            facet: Facet {
                schema_version,
                value,
            },
        });
    }

    pub(crate) fn finish(self) -> Result<ToolingContributionSet, ToolingContributionError> {
        let mut identities = BTreeSet::new();
        let mut resources = Vec::with_capacity(self.resources.len());

        validate_owner(&self.owner)?;

        for resource in self.resources {
            validate_local_id(&resource.id)?;

            if resource.name.trim().is_empty() {
                return Err(ToolingContributionError::EmptyResourceName { id: resource.id });
            }

            if !identities.insert(resource.id.clone()) {
                return Err(ToolingContributionError::DuplicateResource { id: resource.id });
            }

            resources.push(Resource {
                id: qualify(&self.owner, &resource.id),
                kind: ResourceKind::Contribution,
                name: resource.name,
                provenance: Some(Provenance {
                    owner: Some(self.owner.clone()),
                    origin: Some(String::from("tooling-contribution")),
                    ..Provenance::default()
                }),
                labels: resource.labels,
                facets: BTreeMap::new(),
            });
        }

        let mut relationship_identities = BTreeSet::new();
        let mut relationships = Vec::with_capacity(self.relationships.len());

        for relationship in self.relationships {
            validate_endpoint(&relationship.from, &identities)?;
            validate_endpoint(&relationship.to, &identities)?;
            validate_relationship_endpoints(
                &relationship.kind,
                &relationship.from,
                &relationship.to,
            )?;

            let relationship = Relationship {
                kind: relationship.kind,
                from: relationship.from.qualify(&self.owner),
                to: relationship.to.qualify(&self.owner),
                labels: relationship.labels,
            };
            let identity = (
                relationship.kind.clone(),
                relationship.from.clone(),
                relationship.to.clone(),
                relationship.labels.clone(),
            );

            if !relationship_identities.insert(identity) {
                return Err(ToolingContributionError::DuplicateRelationship {
                    from: relationship.from,
                    to: relationship.to,
                });
            }

            relationships.push(relationship);
        }

        let mut facet_identities = BTreeSet::new();
        let mut owner_facets = BTreeMap::new();

        for facet in self.facets {
            validate_local_id(&facet.id)?;

            if facet.facet.schema_version == 0 {
                return Err(ToolingContributionError::InvalidFacetVersion { id: facet.id });
            }

            let target = match facet.resource {
                Some(resource) => {
                    validate_local_id(&resource)?;

                    if !identities.contains(&resource) {
                        return Err(ToolingContributionError::UnknownEndpoint { id: resource });
                    }

                    qualify(&self.owner, &resource)
                }
                None => self.owner.clone(),
            };
            let id = qualify(&target, &facet.id);

            if !facet_identities.insert(id.clone()) {
                return Err(ToolingContributionError::DuplicateFacet { id });
            }

            if target == self.owner {
                owner_facets.insert(id, facet.facet);
            } else {
                let resource = resources
                    .iter_mut()
                    .find(|resource| resource.id == target)
                    .expect("validated contributed resource exists");

                resource.facets.insert(id, facet.facet);
            }
        }

        Ok(ToolingContributionSet {
            owner: self.owner,
            resources,
            relationships,
            owner_facets,
        })
    }
}

/// A structural error in one owner-scoped tooling contribution set.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ToolingContributionError {
    /// A resource, facet, or relationship endpoint has no owner-local identity.
    #[error("tooling contribution contains an empty owner-local identity")]
    EmptyId,
    /// An extension attempted to use a framework/core identity as an owner-local identity.
    #[error("tooling contribution identity '{id}' is reserved by the framework")]
    ReservedId {
        /// Invalid identity.
        id: String,
    },
    /// The collector owner is not a protocol or plugin resource identity.
    #[error("tooling contribution owner '{id}' is not a protocol or plugin identity")]
    InvalidOwner {
        /// Invalid owner identity.
        id: String,
    },
    /// A structurally valid protocol or plugin owner was not projected.
    #[error("tooling contribution owner '{id}' does not exist in the prepared projection")]
    UnknownOwner {
        /// Missing owner identity.
        id: String,
    },
    /// A contributed resource has no display name.
    #[error("tooling resource '{id}' has no name")]
    EmptyResourceName {
        /// Owner-local resource identity.
        id: String,
    },
    /// A contributed facet has no positive schema version.
    #[error("tooling facet '{id}' has schema version zero")]
    InvalidFacetVersion {
        /// Owner-local facet identity.
        id: String,
    },
    /// One collector declared the same owner-local resource more than once.
    #[error("tooling resource '{id}' is declared more than once")]
    DuplicateResource {
        /// Duplicated owner-local identity.
        id: String,
    },
    /// One collector declared the same fully qualified facet more than once.
    #[error("tooling facet '{id}' is declared more than once")]
    DuplicateFacet {
        /// Duplicated fully qualified identity.
        id: String,
    },
    /// One collector declared the same relationship more than once.
    #[error("tooling relationship from '{from}' to '{to}' is declared more than once")]
    DuplicateRelationship {
        /// Qualified source identity.
        from: String,
        /// Qualified target identity.
        to: String,
    },
    /// A relationship or resource facet refers to an undeclared owner-local resource.
    #[error("tooling contribution refers to undeclared owner-local resource '{id}'")]
    UnknownEndpoint {
        /// Missing owner-local identity.
        id: String,
    },
    /// A supported extension relationship uses endpoints incompatible with its semantics.
    #[error("tooling contribution relationship has invalid owner-local endpoints")]
    InvalidRelationshipEndpoints,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
/// Validated owner-qualified resources, relationships, and facets ready for projection.
pub(crate) struct ToolingContributionSet {
    pub(crate) owner: String,
    pub(crate) resources: Vec<Resource>,
    pub(crate) relationships: Vec<Relationship>,
    pub(crate) owner_facets: BTreeMap<String, Facet>,
}

#[derive(Debug)]
/// One owner-local resource awaiting validation and qualification.
struct PendingResource {
    id: String,
    name: String,
    labels: BTreeMap<String, String>,
}

#[derive(Debug)]
/// One owner-local relationship awaiting endpoint validation and qualification.
struct PendingRelationship {
    kind: RelationshipKind,
    from: PendingEndpoint,
    to: PendingEndpoint,
    labels: BTreeMap<String, String>,
}

#[derive(Debug)]
/// One owner-local facet awaiting target validation and qualification.
struct PendingFacet {
    resource: Option<String>,
    id: String,
    facet: Facet,
}

#[derive(Debug)]
/// Internal owned form of a public tooling endpoint.
enum PendingEndpoint {
    Owner,
    Resource(String),
}

impl PendingEndpoint {
    fn qualify(&self, owner: &str) -> String {
        match self {
            Self::Owner => owner.to_string(),
            Self::Resource(id) => qualify(owner, id),
        }
    }
}

impl From<ToolingEndpoint<'_>> for PendingEndpoint {
    fn from(endpoint: ToolingEndpoint<'_>) -> Self {
        match endpoint {
            ToolingEndpoint::Owner => Self::Owner,
            ToolingEndpoint::Resource(id) => Self::Resource(id.to_string()),
        }
    }
}

fn validate_endpoint(
    endpoint: &PendingEndpoint,
    resources: &BTreeSet<String>,
) -> Result<(), ToolingContributionError> {
    let PendingEndpoint::Resource(id) = endpoint else {
        return Ok(());
    };

    validate_local_id(id)?;

    if !resources.contains(id) {
        return Err(ToolingContributionError::UnknownEndpoint { id: id.clone() });
    }

    Ok(())
}

fn validate_relationship_endpoints(
    kind: &RelationshipKind,
    from: &PendingEndpoint,
    to: &PendingEndpoint,
) -> Result<(), ToolingContributionError> {
    let valid = match kind {
        RelationshipKind::Contains => matches!(
            (from, to),
            (
                PendingEndpoint::Owner | PendingEndpoint::Resource(_),
                PendingEndpoint::Resource(_)
            )
        ),
        RelationshipKind::DependsOn
        | RelationshipKind::OrdersBefore
        | RelationshipKind::OrdersAfter => matches!(
            (from, to),
            (PendingEndpoint::Resource(_), PendingEndpoint::Resource(_))
        ),
        _ => false,
    };

    if !valid {
        return Err(ToolingContributionError::InvalidRelationshipEndpoints);
    }

    Ok(())
}

fn validate_local_id(id: &str) -> Result<(), ToolingContributionError> {
    if id.trim().is_empty() {
        return Err(ToolingContributionError::EmptyId);
    }

    if id == "framework"
        || id == "application"
        || [
            "component:",
            "config-binding:",
            "contribution:",
            "hook:",
            "lifecycle:",
            "plugin:",
            "plugin-slot:",
            "protocol:",
            "provider:",
            "scope:",
            "type:",
        ]
        .iter()
        .any(|prefix| id.starts_with(prefix))
    {
        return Err(ToolingContributionError::ReservedId { id: id.to_string() });
    }

    Ok(())
}

fn validate_owner(owner: &str) -> Result<(), ToolingContributionError> {
    let valid = ["plugin:", "protocol:"].iter().any(|prefix| {
        owner.strip_prefix(prefix).is_some_and(|id| {
            let mut parts = id.split('/');

            matches!(
                (parts.next(), parts.next()),
                (Some(namespace), Some(name)) if !namespace.is_empty() && !name.is_empty()
            )
        })
    });

    if !valid {
        return Err(ToolingContributionError::InvalidOwner {
            id: owner.to_string(),
        });
    }

    Ok(())
}

fn qualify(owner: &str, local: &str) -> String {
    let local = local.replace('%', "%25").replace('/', "%2F");

    format!("{owner}/tooling/{local}")
}

#[cfg(test)]
mod tests;
