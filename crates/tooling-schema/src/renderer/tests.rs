use std::collections::BTreeMap;

use semver::VersionReq;
use serde_json::json;

use super::{
    RendererManifest, RendererRequest, RendererResponse, RendererValidationError, RendererView,
    ResourcePresentation,
};
use crate::{
    DocumentIdentity, Facet, Provenance, Resource, ResourceKind, TOOLING_SCHEMA_VERSION,
    ToolingDocument,
};

#[test]
fn manifest_and_response_enforce_exact_owner_and_facet_boundaries() {
    let document = document();
    let manifest = manifest();
    let request = RendererRequest::new(
        &manifest,
        &document,
        RendererView::Inspect,
        document
            .resources
            .iter()
            .map(|resource| resource.id.clone()),
    )
    .expect("request validates");
    let mut response = RendererResponse {
        schema: TOOLING_SCHEMA_VERSION,
        renderer: manifest.id.clone(),
        owner: manifest.owner.clone(),
        presentation: super::RendererPresentation {
            resources: vec![ResourcePresentation {
                resource: String::from("protocol:acme/http/tooling/route/health"),
                label: Some(String::from("GET /health")),
                ..ResourcePresentation::default()
            }],
        },
    };

    let json = response.to_json(&request).expect("response emits");
    let decoded = RendererResponse::from_json(&json, &request).expect("response decodes");

    assert_eq!(decoded, response);

    response.presentation.resources[0].resource = String::from("component:foreign");

    assert!(matches!(
        response.validate(&request),
        Err(RendererValidationError::ForeignResource { .. })
    ));
}

#[test]
fn incompatible_or_foreign_facet_claims_are_rejected_before_invocation() {
    let document = document();
    let mut renderer_manifest = manifest();

    renderer_manifest.facets =
        BTreeMap::from([(String::from("plugin:other/tooling/routes"), vec![1])]);

    assert!(matches!(
        renderer_manifest.validate_document(&document),
        Err(RendererValidationError::ForeignFacet { .. })
    ));

    renderer_manifest = manifest();
    renderer_manifest
        .facets
        .values_mut()
        .next()
        .expect("facet exists")[0] = 2;

    assert!(matches!(
        renderer_manifest.validate_document(&document),
        Err(RendererValidationError::UnsupportedFacetVersion { .. })
    ));
}

#[test]
fn renderer_requests_are_canonical_and_round_trip_only_generic_documents() {
    let document = document();
    let manifest = manifest();
    let request = RendererRequest::new(
        &manifest,
        &document,
        RendererView::Graph,
        [
            String::from("protocol:acme/http/tooling/route/health"),
            String::from("protocol:acme/http"),
        ],
    )
    .expect("request validates");
    let json = request.to_json().expect("request emits");
    let decoded = RendererRequest::from_json(&json).expect("request decodes");

    assert_eq!(decoded.resources[0], "protocol:acme/http");
    assert_eq!(decoded, request);
}

fn manifest() -> RendererManifest {
    RendererManifest {
        schema: TOOLING_SCHEMA_VERSION,
        id: String::from("acme/http"),
        owner: String::from("protocol:acme/http"),
        executable: String::from("bin/http-renderer"),
        document_schema: VersionReq::parse(&format!(
            "^{}.{}",
            TOOLING_SCHEMA_VERSION.major, TOOLING_SCHEMA_VERSION.minor
        ))
        .expect("requirement parses"),
        facets: BTreeMap::from([(String::from("protocol:acme/http/tooling/routes"), vec![1])]),
    }
}

fn document() -> ToolingDocument {
    let owner = String::from("protocol:acme/http");
    let mut owner_resource = Resource {
        id: owner.clone(),
        kind: ResourceKind::Protocol,
        name: String::from("HTTP"),
        ..Resource::default()
    };

    owner_resource.facets.insert(
        String::from("protocol:acme/http/tooling/routes"),
        Facet {
            schema_version: 1,
            value: json!({"count": 1}),
        },
    );

    let mut document = ToolingDocument::new(
        "0.20.0",
        DocumentIdentity {
            application: String::from("fixture"),
            ..DocumentIdentity::default()
        },
        "acme/http",
    );

    document.resources = vec![
        owner_resource,
        Resource {
            id: String::from("protocol:acme/http/tooling/route/health"),
            kind: ResourceKind::Contributor,
            name: String::from("health"),
            provenance: Some(Provenance {
                owner: Some(owner),
                ..Provenance::default()
            }),
            ..Resource::default()
        },
        Resource {
            id: String::from("component:foreign"),
            kind: ResourceKind::Component,
            name: String::from("Foreign"),
            ..Resource::default()
        },
    ];
    document.canonicalize();

    document
}
