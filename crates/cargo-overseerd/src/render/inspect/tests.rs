use std::collections::BTreeMap;

use overseerd_tooling_schema::{
    CliCommand, CliMetadata, CliOwner, CliProvider, CliProviderKind, DocumentIdentity, Facet,
    Provenance, Relationship, RelationshipKind, Resource, ResourceKind, ToolingDocument,
};
use serde_json::json;

use super::{selected_resource_ids, write_inspection, write_inspection_with_presentation};
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
fn renderer_presentation_improves_text_without_hiding_generic_identity() {
    let document = fixture();
    let resource = document.resources[0].id.clone();
    let presentation = overseerd_tooling_schema::renderer::RendererPresentation {
        resources: vec![overseerd_tooling_schema::renderer::ResourcePresentation {
            resource: resource.clone(),
            label: Some(String::from("HTTP GET /health")),
            summary: Some(String::from("health endpoint")),
            details: std::collections::BTreeMap::from([(
                String::from("method"),
                String::from("GET"),
            )]),
            ..Default::default()
        }],
    };
    let mut output = Vec::new();

    write_inspection_with_presentation(
        &document,
        &InspectFilters::default(),
        Some(&presentation),
        false,
        &mut output,
    )
    .expect("inspection writes");

    let output = String::from_utf8(output).expect("inspection is UTF-8");

    assert!(output.contains(&format!("HTTP GET /health ({resource})")));
    assert!(output.contains("renderer summary: health endpoint"));
    assert!(output.contains("renderer method: GET"));
}

#[test]
fn renderer_selection_matches_active_inspection_filters() {
    let document = fixture();
    let selected = selected_resource_ids(
        &document,
        &InspectFilters {
            resources: vec![document.resources[0].id.clone()],
            ..InspectFilters::default()
        },
    );

    assert_eq!(selected, [document.resources[0].id.clone()]);
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
fn inspection_escapes_controls_in_identity_resources_and_labels() {
    let mut document = fixture();

    document.identity.application = String::from("fixture\u{1b}[31m\nnext\rline");
    document.resources[2].name = String::from("Worker\u{1b}\nname");
    document.resources[2].labels.insert(
        String::from("hostile\nlabel"),
        String::from("value\r\u{1b}"),
    );
    let mut output = Vec::new();

    write_inspection(&document, &InspectFilters::default(), false, &mut output)
        .expect("inspection writes");

    let output = String::from_utf8(output).expect("inspection is UTF-8");

    assert!(!output.contains('\u{1b}'));
    assert!(!output.contains('\r'));
    assert!(output.contains(r"name: fixture\u{1b}[31m\nnext\rline"));
    assert!(output.contains(r"Worker\u{1b}\nname"));
    assert!(output.contains(r"hostile\nlabel: value\r\u{1b}"));
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

#[test]
fn resource_names_select_cli_provider_resources_consistently() {
    let document = cli_fixture(ResourceKind::Plugin);

    for resource in ["Worker Plugin", "Worker CLI Contribution"] {
        let filters = InspectFilters {
            resources: vec![String::from(resource)],
            ..InspectFilters::default()
        };
        let mut output = Vec::new();

        write_inspection(&document, &filters, false, &mut output).expect("inspection writes");

        let output = String::from_utf8(output).expect("inspection is UTF-8");

        assert!(output.contains("provider cli-provider:plugin:test/worker:test/worker-cli"));
    }
}

#[test]
fn plugin_filter_excludes_non_plugin_cli_contributors() {
    let document = cli_fixture(ResourceKind::Protocol);
    let filters = InspectFilters {
        plugins: vec![String::from("Worker Protocol")],
        ..InspectFilters::default()
    };
    let mut output = Vec::new();

    write_inspection(&document, &filters, false, &mut output).expect("inspection writes");

    let output = String::from_utf8(output).expect("inspection is UTF-8");

    assert!(output.contains("CLI\n  none"));
    assert!(!output.contains("provider cli-provider:plugin:test/worker:test/worker-cli"));
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

fn cli_fixture(contributor_kind: ResourceKind) -> ToolingDocument {
    let mut document = fixture();

    document.resources[0].kind = contributor_kind;
    document.resources[0].name = if matches!(document.resources[0].kind, ResourceKind::Plugin) {
        String::from("Worker Plugin")
    } else {
        String::from("Worker Protocol")
    };
    document.resources.push(Resource {
        id: String::from("contribution:plugin:test/worker:test/worker-cli"),
        kind: ResourceKind::Contribution,
        name: String::from("Worker CLI Contribution"),
        provenance: Some(Provenance {
            owner: Some(String::from("plugin:test/worker")),
            ..Provenance::default()
        }),
        ..Resource::default()
    });
    document.cli = Some(CliMetadata {
        root: CliCommand {
            name: String::from("fixture"),
            owner: CliOwner::Application,
            ..CliCommand::default()
        },
        providers: vec![CliProvider {
            id: String::from("cli-provider:plugin:test/worker:test/worker-cli"),
            contributor: String::from("plugin:test/worker"),
            contribution: String::from("test/worker-cli"),
            kind: CliProviderKind::Command,
        }],
        default_command: None,
    });

    document
}
