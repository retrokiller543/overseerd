use std::collections::BTreeMap;

use overseerd_tooling_schema::{
    DocumentIdentity, Facet, Provenance, Relationship, RelationshipKind, Resource, ResourceKind,
    ToolingDocument,
};
use serde_json::json;

use super::write_inspection;
use crate::cli::{InspectFilters, InspectResourceKind};

#[test]
fn generic_inspection_renders_unknown_facets_and_provenance() {
    let document = fixture();
    let mut output = Vec::new();

    write_inspection(&document, &InspectFilters::default(), false, &mut output)
        .expect("inspection writes");

    let output = String::from_utf8(output).expect("inspection is UTF-8");

    assert!(output.contains("Application\n  name: fixture"));
    assert!(output.contains("component Worker (component:worker)"));
    assert!(output.contains("owner: plugin:test/worker"));
    assert!(output.contains("facet third-party/routes@2"));
    assert!(output.contains("component:worker --depends-on--> scope:test/request"));
}

#[test]
fn inspection_filters_dimensions_with_and_and_values_with_or() {
    let document = fixture();
    let filters = InspectFilters {
        kinds: vec![InspectResourceKind::Component, InspectResourceKind::Scope],
        resources: vec![String::from("Worker"), String::from("Request")],
        plugins: vec![String::from("test/worker")],
        ..InspectFilters::default()
    };
    let mut output = Vec::new();

    write_inspection(&document, &filters, false, &mut output).expect("inspection writes");

    let output = String::from_utf8(output).expect("inspection is UTF-8");

    assert!(output.contains("component Worker (component:worker)"));
    assert!(!output.contains("scope Request (scope:test/request)"));
    assert!(!output.contains("plugin test/worker (plugin:test/worker)"));
}

#[test]
fn colored_inspection_decorates_only_headings() {
    let document = fixture();
    let mut output = Vec::new();

    write_inspection(&document, &InspectFilters::default(), true, &mut output)
        .expect("inspection writes");

    let output = String::from_utf8(output).expect("inspection is UTF-8");

    assert!(output.starts_with("\u{1b}[1;36mApplication\u{1b}[0m\n"));
    assert!(output.contains("component Worker (component:worker)"));
}

#[test]
fn scope_name_filter_includes_assigned_resources() {
    let document = fixture();
    let filters = InspectFilters {
        scopes: vec![String::from("Request")],
        ..InspectFilters::default()
    };
    let mut output = Vec::new();

    write_inspection(&document, &filters, false, &mut output).expect("inspection writes");

    let output = String::from_utf8(output).expect("inspection is UTF-8");

    assert!(output.contains("scope Request (scope:test/request)"));
    assert!(output.contains("component Worker (component:worker)"));
}

fn fixture() -> ToolingDocument {
    let mut document = ToolingDocument::new(
        "0.20.0",
        DocumentIdentity {
            application: String::from("fixture"),
            ..DocumentIdentity::default()
        },
        "test/protocol",
    );

    document.resources = vec![
        Resource {
            id: String::from("plugin:test/worker"),
            kind: ResourceKind::Plugin,
            name: String::from("test/worker"),
            ..Resource::default()
        },
        Resource {
            id: String::from("scope:test/request"),
            kind: ResourceKind::Scope,
            name: String::from("Request"),
            labels: BTreeMap::from([(String::from("scope-id"), String::from("test/request"))]),
            ..Resource::default()
        },
        Resource {
            id: String::from("component:worker"),
            kind: ResourceKind::Component,
            name: String::from("Worker"),
            provenance: Some(Provenance {
                owner: Some(String::from("plugin:test/worker")),
                origin: Some(String::from("application-plugin")),
                ..Provenance::default()
            }),
            labels: BTreeMap::from([(String::from("scope"), String::from("test/request"))]),
            facets: BTreeMap::from([(
                String::from("third-party/routes"),
                Facet {
                    schema_version: 2,
                    value: json!({"path": "/worker"}),
                },
            )]),
        },
    ];
    document.relationships.push(Relationship {
        kind: RelationshipKind::DependsOn,
        from: String::from("component:worker"),
        to: String::from("scope:test/request"),
        labels: BTreeMap::new(),
    });

    document
}
