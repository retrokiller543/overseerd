use std::collections::{BTreeMap, BTreeSet};

use overseerd_tooling_schema::{
    CliArgument, CliCommand, CliMetadata, CliOwner, CliProvider, CliProviderKind, Diagnostic,
    DiagnosticSeverity, DocumentIdentity, Provenance, Relationship, RelationshipKind, Resource,
    ResourceKind, ToolingDocument,
};

use super::{
    GraphDirection, GraphQuery, GraphQueryError, GraphRelationFamily, GraphSelectorKind,
    ResourceExplanation, explain_resource, query_graph,
};

#[test]
fn exact_selectors_use_id_precedence_and_report_sorted_name_ambiguity() {
    let mut document = document();

    document.resources.extend([
        resource("component:id-wins", ResourceKind::Component, "Shared"),
        resource("component:a", ResourceKind::Component, "Shared"),
        resource(
            "component:name-collision",
            ResourceKind::Component,
            "component:id-wins",
        ),
    ]);

    let view = query_graph(
        &document,
        &GraphQuery {
            resources: vec![String::from("component:id-wins")],
            ..GraphQuery::default()
        },
    )
    .expect("stable ID selects before an equal name");

    assert_eq!(view.roots, ["component:id-wins"]);
    assert_eq!(node_ids(&view), BTreeSet::from(["component:id-wins"]));

    let error = explain_resource(&document, "Shared").expect_err("duplicate name is ambiguous");
    let GraphQueryError::Ambiguous {
        kind,
        selector,
        candidates,
    } = error
    else {
        panic!("unexpected selector result");
    };

    assert_eq!(kind, GraphSelectorKind::Resource);
    assert_eq!(selector, "Shared");
    assert_eq!(candidates, ["component:a", "component:id-wins"]);
}

#[test]
fn exact_unique_name_and_not_found_resolution_are_typed() {
    let mut document = document();

    document.resources.push(resource(
        "component:worker",
        ResourceKind::Component,
        "Worker",
    ));

    let explanation = ResourceExplanation::query(&document, "Worker").expect("name selects");

    assert_eq!(explanation.resource.id, "component:worker");

    let error = explain_resource(&document, "worker").expect_err("matching is case-sensitive");

    assert!(matches!(
        error,
        GraphQueryError::NotFound {
            kind: GraphSelectorKind::Resource,
            selector,
        } if selector == "worker"
    ));
}

#[test]
fn dependency_traversal_uses_semantic_orientation_and_excludes_scope_placement() {
    let mut document = document();
    let mut scope_edge = relationship(
        RelationshipKind::DependsOn,
        "component:consumer",
        "scope:request",
    );

    scope_edge
        .labels
        .insert(String::from("role"), String::from("scope"));
    document.resources.extend([
        resource("component:consumer", ResourceKind::Component, "Consumer"),
        resource("type:service", ResourceKind::Type, "Service"),
        resource(
            "provider:service",
            ResourceKind::Provider,
            "Service Provider",
        ),
        resource(
            "config-binding:service",
            ResourceKind::ConfigBinding,
            "Service Config",
        ),
        resource(
            "component:implementation",
            ResourceKind::Component,
            "Implementation",
        ),
        resource("scope:request", ResourceKind::Scope, "Request"),
    ]);
    document.relationships.extend([
        relationship(
            RelationshipKind::DependsOn,
            "component:consumer",
            "type:service",
        ),
        relationship(
            RelationshipKind::Provides,
            "provider:service",
            "type:service",
        ),
        relationship(
            RelationshipKind::Binds,
            "config-binding:service",
            "type:service",
        ),
        relationship(
            RelationshipKind::Provides,
            "component:implementation",
            "provider:service",
        ),
        scope_edge,
    ]);

    let upstream = query_graph(
        &document,
        &GraphQuery {
            resources: vec![String::from("component:consumer")],
            family: GraphRelationFamily::Dependencies,
            direction: GraphDirection::Upstream,
            ..GraphQuery::default()
        },
    )
    .expect("dependency graph resolves");

    assert_eq!(
        node_ids(&upstream),
        BTreeSet::from([
            "component:consumer",
            "component:implementation",
            "config-binding:service",
            "provider:service",
            "type:service",
        ])
    );
    assert!(!node_ids(&upstream).contains("scope:request"));

    let downstream = query_graph(
        &document,
        &GraphQuery {
            resources: vec![String::from("component:implementation")],
            family: GraphRelationFamily::Dependencies,
            direction: GraphDirection::Downstream,
            ..GraphQuery::default()
        },
    )
    .expect("reverse dependency graph resolves");

    assert!(node_ids(&downstream).contains("component:consumer"));
}

#[test]
fn contributor_and_plugin_selectors_include_owned_and_contained_closure() {
    let mut document = document();
    let mut owned = resource(
        "contribution:plugin:worker:feature",
        ResourceKind::Contribution,
        "feature",
    );
    let mut component = resource("component:worker", ResourceKind::Component, "Worker");

    owned.provenance = Some(provenance("plugin:worker"));
    component.provenance = Some(provenance("plugin:worker"));
    document.resources.extend([
        resource("plugin:worker", ResourceKind::Plugin, "Worker Plugin"),
        owned,
        resource(
            "contribution:plugin:worker:nested",
            ResourceKind::Contribution,
            "nested",
        ),
        component,
    ]);
    document.relationships.extend([
        relationship(
            RelationshipKind::Contains,
            "plugin:worker",
            "contribution:plugin:worker:feature",
        ),
        relationship(
            RelationshipKind::Contains,
            "contribution:plugin:worker:feature",
            "contribution:plugin:worker:nested",
        ),
        relationship(
            RelationshipKind::Contributes,
            "contribution:plugin:worker:nested",
            "component:worker",
        ),
    ]);

    let plugin_view = query_graph(
        &document,
        &GraphQuery {
            plugins: vec![String::from("Worker Plugin")],
            family: GraphRelationFamily::Ownership,
            direction: GraphDirection::Downstream,
            ..GraphQuery::default()
        },
    )
    .expect("plugin closure resolves");
    let contributor_view = query_graph(
        &document,
        &GraphQuery {
            contributors: vec![String::from("plugin:worker")],
            family: GraphRelationFamily::Ownership,
            direction: GraphDirection::Downstream,
            ..GraphQuery::default()
        },
    )
    .expect("contributor closure resolves");

    assert_eq!(node_ids(&plugin_view), node_ids(&contributor_view));
    assert_eq!(
        node_ids(&plugin_view),
        BTreeSet::from([
            "component:worker",
            "contribution:plugin:worker:feature",
            "contribution:plugin:worker:nested",
            "plugin:worker",
        ])
    );
}

#[test]
fn traversal_is_iterative_cycle_safe_and_conflicts_are_symmetric() {
    let mut document = document();

    document.resources.extend([
        resource("plugin:a", ResourceKind::Plugin, "A"),
        resource("plugin:b", ResourceKind::Plugin, "B"),
        resource("plugin:c", ResourceKind::Plugin, "C"),
    ]);
    document.relationships.extend([
        relationship(RelationshipKind::DependsOn, "plugin:a", "plugin:b"),
        relationship(RelationshipKind::DependsOn, "plugin:b", "plugin:a"),
        relationship(RelationshipKind::Conflicts, "plugin:b", "plugin:c"),
    ]);

    for direction in [GraphDirection::Upstream, GraphDirection::Downstream] {
        let view = query_graph(
            &document,
            &GraphQuery {
                resources: vec![String::from("plugin:c")],
                family: GraphRelationFamily::Composition,
                direction,
                ..GraphQuery::default()
            },
        )
        .expect("cyclic composition graph terminates");

        assert_eq!(
            node_ids(&view),
            BTreeSet::from(["plugin:a", "plugin:b", "plugin:c"])
        );
    }
}

#[test]
fn scope_lifecycle_and_ownership_families_follow_documented_orientation() {
    let mut document = document();

    document.resources.extend([
        resource("scope:root", ResourceKind::Scope, "Root"),
        resource("scope:request", ResourceKind::Scope, "Request"),
        resource("component:worker", ResourceKind::Component, "Worker"),
        resource("hook:startup", ResourceKind::Hook, "Startup Hook"),
        resource("lifecycle:startup", ResourceKind::Lifecycle, "Startup"),
        resource("lifecycle:serve", ResourceKind::Lifecycle, "Serve"),
        resource("plugin:worker", ResourceKind::Plugin, "Worker Plugin"),
        resource(
            "contribution:plugin:worker:route",
            ResourceKind::Contribution,
            "route",
        ),
    ]);
    document.relationships.extend([
        relationship(RelationshipKind::OpensScope, "scope:root", "scope:request"),
        relationship(RelationshipKind::Hooks, "component:worker", "hook:startup"),
        relationship(RelationshipKind::Hooks, "hook:startup", "lifecycle:startup"),
        relationship(
            RelationshipKind::OrdersBefore,
            "lifecycle:startup",
            "lifecycle:serve",
        ),
        relationship(
            RelationshipKind::Contains,
            "plugin:worker",
            "contribution:plugin:worker:route",
        ),
    ]);

    for (root, family, expected) in [
        ("scope:request", GraphRelationFamily::Scopes, "scope:root"),
        (
            "lifecycle:serve",
            GraphRelationFamily::Lifecycle,
            "component:worker",
        ),
        (
            "contribution:plugin:worker:route",
            GraphRelationFamily::Ownership,
            "plugin:worker",
        ),
    ] {
        let view = query_graph(
            &document,
            &GraphQuery {
                resources: vec![root.to_string()],
                family,
                direction: GraphDirection::Upstream,
                ..GraphQuery::default()
            },
        )
        .expect("family traversal resolves");

        assert!(node_ids(&view).contains(expected));
    }
}

#[test]
fn no_selectors_returns_disconnected_full_graph_with_filtered_edges() {
    let mut document = document();

    document.resources.extend([
        resource("component:a", ResourceKind::Component, "A"),
        resource("type:a", ResourceKind::Type, "A Type"),
        resource("component:isolated", ResourceKind::Component, "Isolated"),
        resource("scope:root", ResourceKind::Scope, "Root"),
    ]);
    document.relationships.extend([
        relationship(RelationshipKind::Provides, "component:a", "type:a"),
        relationship(
            RelationshipKind::DependsOn,
            "component:isolated",
            "scope:root",
        ),
    ]);
    document.relationships[1]
        .labels
        .insert(String::from("role"), String::from("scope"));
    document.diagnostics.extend([
        Diagnostic {
            code: String::from("fixture/unattributed"),
            severity: DiagnosticSeverity::Warning,
            message: String::from("global validation context"),
            ..Diagnostic::default()
        },
        Diagnostic {
            code: String::from("fixture/component-a"),
            severity: DiagnosticSeverity::Info,
            message: String::from("component context"),
            resources: vec![String::from("component:a")],
            ..Diagnostic::default()
        },
    ]);

    let view = GraphQuery {
        family: GraphRelationFamily::Dependencies,
        ..GraphQuery::default()
    }
    .execute(&document)
    .expect("full graph resolves");

    assert_eq!(view.nodes.len(), document.resources.len());
    assert_eq!(view.edges.len(), 1);
    assert_eq!(view.diagnostics.len(), 2);
    assert!(view.roots.is_empty());
}

#[test]
fn filtered_successful_graph_retains_selected_and_unattributed_diagnostics() {
    let mut document = rich_document();

    document.diagnostics.push(Diagnostic {
        code: String::from("fixture/global-context"),
        severity: DiagnosticSeverity::Info,
        message: String::from("global validation context"),
        ..Diagnostic::default()
    });

    let view = query_graph(
        &document,
        &GraphQuery {
            resources: vec![String::from("component:worker")],
            ..GraphQuery::default()
        },
    )
    .expect("filtered graph resolves");
    let codes = view
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();

    assert_eq!(codes, ["fixture/global-context", "fixture/worker-warning"]);
}

#[test]
fn canonical_output_is_identical_under_input_permutations() {
    let mut first = rich_document();
    let mut second = first.clone();

    second.resources.reverse();
    second.relationships.reverse();
    second.diagnostics.reverse();
    second
        .cli
        .as_mut()
        .expect("CLI fixture exists")
        .providers
        .reverse();

    let query = GraphQuery {
        contributors: vec![String::from("plugin:worker")],
        ..GraphQuery::default()
    };
    let first_json = query
        .execute(&first)
        .expect("first graph resolves")
        .to_canonical_json()
        .expect("first graph serializes");
    let second_json = query
        .execute(&second)
        .expect("second graph resolves")
        .to_canonical_json()
        .expect("second graph serializes");

    assert_eq!(first_json, second_json);

    first.canonicalize();
    assert!(first_json.contains(&format!("\"schema\":\"{}\"", super::TOOLING_SCHEMA_VERSION)));
}

#[test]
fn commas_and_hostile_selector_strings_remain_whole_exact_values() {
    let mut document = document();

    document.resources.extend([
        resource("component:a,b", ResourceKind::Component, "Comma, Name"),
        resource("component:a", ResourceKind::Component, "A"),
        resource("component:b", ResourceKind::Component, "B"),
    ]);

    for selector in ["component:a,b", "Comma, Name"] {
        let explanation = explain_resource(&document, selector).expect("whole selector resolves");

        assert_eq!(explanation.resource.id, "component:a,b");
    }

    for selector in ["*", "component:a,b\ncomponent:b", "../component:a", ""] {
        let error = explain_resource(&document, selector).expect_err("selector stays exact");

        assert!(matches!(error, GraphQueryError::NotFound { .. }));
    }
}

#[test]
fn explanation_reports_cli_providers_and_parser_ownership_without_winner_inference() {
    let document = rich_document();
    let contributor = explain_resource(&document, "plugin:worker").expect("plugin explains");
    let contribution = explain_resource(&document, "worker-cli").expect("contribution explains");

    for explanation in [&contributor, &contribution] {
        assert_eq!(explanation.cli_providers.len(), 1);
        assert_eq!(
            explanation.cli_providers[0].id,
            "cli-provider:plugin:worker:worker-cli"
        );
        assert_eq!(explanation.cli_ownership.commands.len(), 1);
        assert_eq!(
            explanation.cli_ownership.commands[0].path,
            ["fixture", "worker"]
        );
        assert_eq!(explanation.cli_ownership.arguments.len(), 1);
        assert_eq!(explanation.cli_ownership.arguments[0].id, "threads");
    }

    let json = contributor
        .to_canonical_json()
        .expect("explanation serializes");

    assert!(!json.contains("winner"));
    assert!(json.contains("resolved-provider"));
}

#[test]
fn cli_provider_ownership_requires_the_exact_canonical_contribution_id() {
    let mut document = rich_document();
    let mut collision = resource(
        "contribution:plugin:worker:not-worker-cli",
        ResourceKind::Contribution,
        "worker-cli",
    );

    collision.provenance = Some(provenance("plugin:worker"));
    document.resources.push(collision);

    let exact = explain_resource(&document, "contribution:plugin:worker:worker-cli")
        .expect("canonical contribution explains");
    let collision = explain_resource(&document, "contribution:plugin:worker:not-worker-cli")
        .expect("colliding contribution explains");

    assert_eq!(exact.cli_providers.len(), 1);
    assert!(collision.cli_providers.is_empty());
    assert!(collision.cli_ownership.commands.is_empty());
    assert!(collision.cli_ownership.arguments.is_empty());
}

#[test]
fn every_emitted_edge_has_both_endpoint_nodes_and_diagnostics_are_relevant() {
    let document = rich_document();
    let view = query_graph(
        &document,
        &GraphQuery {
            resources: vec![String::from("component:worker")],
            direction: GraphDirection::Both,
            ..GraphQuery::default()
        },
    )
    .expect("selected graph resolves");
    let ids = node_ids(&view);

    assert!(
        view.edges
            .iter()
            .all(|edge| ids.contains(edge.from.as_str()) && ids.contains(edge.to.as_str()))
    );
    assert_eq!(view.diagnostics.len(), 1);
    assert_eq!(view.diagnostics[0].code, "fixture/worker-warning");
}

#[test]
fn successful_graph_is_explicitly_complete_with_available_source_facts() {
    let view = query_graph(&document(), &GraphQuery::default()).expect("graph resolves");

    assert!(view.complete);
    assert_eq!(view.failure_phase, None);
    assert_eq!(view.source.framework_version.as_deref(), Some("0.20.0"));
    assert_eq!(view.source.protocol.as_deref(), Some("test/protocol"));
}

fn rich_document() -> ToolingDocument {
    let mut document = document();
    let mut contribution = resource(
        "contribution:plugin:worker:worker-cli",
        ResourceKind::Contribution,
        "worker-cli",
    );
    let mut component = resource("component:worker", ResourceKind::Component, "Worker");
    let provider_id = "cli-provider:plugin:worker:worker-cli";

    contribution.provenance = Some(provenance("plugin:worker"));
    component.provenance = Some(provenance("plugin:worker"));
    document.resources.extend([
        resource("plugin:worker", ResourceKind::Plugin, "Worker Plugin"),
        contribution,
        component,
        resource("type:worker", ResourceKind::Type, "Worker Type"),
    ]);
    document.relationships.extend([
        relationship_with_labels(
            RelationshipKind::Contributes,
            "plugin:worker",
            "contribution:plugin:worker:worker-cli",
            [(String::from("role"), String::from("resolved-provider"))],
        ),
        relationship(
            RelationshipKind::Contributes,
            "contribution:plugin:worker:worker-cli",
            "component:worker",
        ),
        relationship(
            RelationshipKind::Provides,
            "component:worker",
            "type:worker",
        ),
    ]);
    document.diagnostics.extend([
        Diagnostic {
            code: String::from("fixture/unrelated"),
            severity: DiagnosticSeverity::Info,
            message: String::from("unrelated"),
            resources: vec![String::from("application")],
            ..Diagnostic::default()
        },
        Diagnostic {
            code: String::from("fixture/worker-warning"),
            severity: DiagnosticSeverity::Warning,
            message: String::from("worker warning"),
            resources: vec![String::from("component:worker")],
            ..Diagnostic::default()
        },
    ]);
    document.cli = Some(CliMetadata {
        root: CliCommand {
            name: String::from("fixture"),
            owner: CliOwner::Application,
            commands: vec![CliCommand {
                name: String::from("worker"),
                owner: CliOwner::Plugin {
                    provider: String::from(provider_id),
                },
                arguments: vec![CliArgument {
                    id: String::from("threads"),
                    long: Some(String::from("threads")),
                    owner: CliOwner::Plugin {
                        provider: String::from(provider_id),
                    },
                    ..CliArgument::default()
                }],
                ..CliCommand::default()
            }],
            ..CliCommand::default()
        },
        providers: vec![CliProvider {
            id: String::from(provider_id),
            contributor: String::from("plugin:worker"),
            contribution: String::from("worker-cli"),
            kind: CliProviderKind::Command,
        }],
        default_command: None,
    });

    document
}

fn document() -> ToolingDocument {
    let mut document = ToolingDocument::new(
        "0.20.0",
        DocumentIdentity {
            application: String::from("fixture"),
            ..DocumentIdentity::default()
        },
        "test/protocol",
    );

    document.resources.push(resource(
        "application",
        ResourceKind::Application,
        "fixture",
    ));

    document
}

fn resource(id: &str, kind: ResourceKind, name: &str) -> Resource {
    Resource {
        id: id.to_string(),
        kind,
        name: name.to_string(),
        ..Resource::default()
    }
}

fn relationship(kind: RelationshipKind, from: &str, to: &str) -> Relationship {
    Relationship {
        kind,
        from: from.to_string(),
        to: to.to_string(),
        labels: BTreeMap::new(),
    }
}

fn relationship_with_labels(
    kind: RelationshipKind,
    from: &str,
    to: &str,
    labels: impl IntoIterator<Item = (String, String)>,
) -> Relationship {
    Relationship {
        labels: labels.into_iter().collect(),
        ..relationship(kind, from, to)
    }
}

fn provenance(owner: &str) -> Provenance {
    Provenance {
        owner: Some(owner.to_string()),
        ..Provenance::default()
    }
}

fn node_ids(view: &super::GraphView) -> BTreeSet<&str> {
    view.nodes
        .iter()
        .map(|resource| resource.id.as_str())
        .collect()
}
