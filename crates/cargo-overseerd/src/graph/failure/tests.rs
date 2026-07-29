use std::collections::BTreeSet;

use overseerd_tooling_schema::{
    Diagnostic, DiagnosticSeverity, DocumentIdentity, ProbeFailure, ResourceKind,
};
use semver::Version;

use super::query_failure_graph;
use crate::graph::{
    GraphDirection, GraphQuery, GraphQueryError, GraphRelationFamily, GraphSelectorKind, GraphView,
};

#[test]
fn preserves_missing_provider_and_invalid_scope_identities() {
    let failure = failure(
        Some("prepare"),
        vec![diagnostic(
            "fixture/missing-resources",
            "provider:missing\u{1b}[31m",
            ["scope:invalid parent", "provider:missing\u{1b}[31m"],
        )],
    );
    let view = failure_graph(&failure, &GraphQuery::default()).expect("failure graph resolves");

    assert!(!view.complete);
    assert_eq!(view.failure_phase.as_deref(), Some("prepare"));
    assert_eq!(view.source.framework_version, None);
    assert_eq!(view.source.protocol, None);
    assert!(view.edges.is_empty());
    assert_eq!(
        view.nodes
            .iter()
            .map(|node| (node.id.as_str(), node.kind.clone()))
            .collect::<Vec<_>>(),
        [
            ("provider:missing\u{1b}[31m", ResourceKind::Provider),
            ("scope:invalid parent", ResourceKind::Scope),
        ]
    );
    assert!(view.nodes.iter().all(|node| {
        node.labels.get("graph-status").map(String::as_str) == Some("diagnostic-only")
            && node.labels.get("graph-completeness").map(String::as_str) == Some("incomplete")
    }));
    assert_eq!(view.diagnostics[0].message, "provider:missing\u{1b}[31m");
    assert_eq!(view.diagnostics[0].fix.as_deref(), Some("repair it"));
}

#[test]
fn scope_violation_graph_has_component_type_and_scope_nodes() {
    let failure = failure(
        Some("prepare"),
        vec![diagnostic(
            "overseerd/tooling-scope-violation",
            "scope violation",
            [
                "component:consumer-component",
                "type:fixture::Dependency",
                "scope:fixture/request",
                "scope:fixture/connection",
            ],
        )],
    );
    let view = failure_graph(&failure, &GraphQuery::default()).expect("failure graph resolves");

    assert_eq!(
        view.nodes
            .iter()
            .map(|node| (node.id.as_str(), node.kind.clone()))
            .collect::<Vec<_>>(),
        [
            ("component:consumer-component", ResourceKind::Component),
            ("scope:fixture/connection", ResourceKind::Scope),
            ("scope:fixture/request", ResourceKind::Scope),
            ("type:fixture::Dependency", ResourceKind::Type),
        ]
    );
}

#[test]
fn scope_unreachable_graph_has_consumer_type_scope_and_provider_nodes() {
    let failure = failure(
        Some("prepare"),
        vec![diagnostic(
            "overseerd/tooling-scope-unreachable",
            "registered provider is unreachable",
            [
                "component:consumer-component",
                "type:fixture::Service",
                "scope:fixture/request",
                "provider:fixture::Service:fixture::Provider:primary",
                "component:provider-component",
                "type:fixture::Provider",
                "scope:fixture/sibling",
            ],
        )],
    );
    let view = failure_graph(&failure, &GraphQuery::default()).expect("failure graph resolves");

    assert_eq!(
        view.nodes
            .iter()
            .map(|node| (node.id.as_str(), node.kind.clone()))
            .collect::<Vec<_>>(),
        [
            ("component:consumer-component", ResourceKind::Component),
            ("component:provider-component", ResourceKind::Component),
            (
                "provider:fixture::Service:fixture::Provider:primary",
                ResourceKind::Provider,
            ),
            ("scope:fixture/request", ResourceKind::Scope),
            ("scope:fixture/sibling", ResourceKind::Scope),
            ("type:fixture::Provider", ResourceKind::Type),
            ("type:fixture::Service", ResourceKind::Type),
        ]
    );
}

#[test]
fn orphan_provider_graph_has_provider_component_and_type_nodes() {
    let failure = failure(
        Some("prepare"),
        vec![diagnostic(
            "overseerd/tooling-provider-component-missing",
            "provider component is absent",
            [
                "provider:fixture::Service:fixture::Provider:primary",
                "component:fixture::Provider",
                "type:fixture::Service",
                "type:fixture::Provider",
            ],
        )],
    );
    let view = failure_graph(&failure, &GraphQuery::default()).expect("failure graph resolves");

    assert_eq!(
        view.nodes
            .iter()
            .map(|node| (node.id.as_str(), node.kind.clone()))
            .collect::<Vec<_>>(),
        [
            ("component:fixture::Provider", ResourceKind::Component),
            (
                "provider:fixture::Service:fixture::Provider:primary",
                ResourceKind::Provider,
            ),
            ("type:fixture::Provider", ResourceKind::Type),
            ("type:fixture::Service", ResourceKind::Type),
        ]
    );
}

#[test]
fn diagnostic_without_resources_gets_stable_attached_placeholder() {
    let failure = failure(
        None,
        vec![Diagnostic {
            code: String::from("fixture/unattributed"),
            severity: DiagnosticSeverity::Error,
            message: String::from("no resource identity"),
            resources: Vec::new(),
            sources: Vec::new(),
            fix: Some(String::from("repair it")),
        }],
    );
    let view = failure_graph(&failure, &GraphQuery::default()).expect("failure graph resolves");

    assert_eq!(view.nodes.len(), 1);
    assert_eq!(view.nodes[0].id, "diagnostic:unattributed:0000");
    assert_eq!(view.nodes[0].kind, ResourceKind::Type);
    assert_eq!(view.diagnostics[0].resources, [view.nodes[0].id.clone()]);
}

#[test]
fn output_is_deterministic_under_diagnostic_and_resource_permutations() {
    let first = failure(
        Some("configure"),
        vec![
            diagnostic("fixture/b", "second", ["plugin:b", "scope:z"]),
            diagnostic("fixture/a", "first", ["provider:a", "plugin:b"]),
        ],
    );
    let mut second = first.clone();

    second.diagnostics.reverse();
    second.diagnostics[0].resources.reverse();
    second.diagnostics[1].resources.reverse();

    let first = failure_graph(&first, &GraphQuery::default())
        .expect("first graph resolves")
        .to_canonical_json()
        .expect("first graph serializes");
    let second = failure_graph(&second, &GraphQuery::default())
        .expect("second graph resolves")
        .to_canonical_json()
        .expect("second graph serializes");

    assert_eq!(first, second);
}

#[test]
fn exact_selectors_filter_nodes_and_report_typed_misuse() {
    let failure = failure(
        Some("prepare"),
        vec![
            diagnostic("fixture/plugin", "plugin", ["plugin:worker"]),
            diagnostic("fixture/component", "component", ["component:worker"]),
            diagnostic("fixture/framework", "framework", ["framework"]),
        ],
    );
    let view = failure_graph(
        &failure,
        &GraphQuery {
            resources: vec![
                String::from("component:worker"),
                String::from("component:worker"),
            ],
            contributors: vec![String::from("framework")],
            plugins: vec![String::from("plugin:worker")],
            family: GraphRelationFamily::Composition,
            direction: GraphDirection::Upstream,
        },
    )
    .expect("eligible exact selectors resolve");

    assert_eq!(
        node_ids(&view),
        BTreeSet::from(["component:worker", "framework", "plugin:worker"])
    );
    assert_eq!(
        view.roots,
        ["component:worker", "framework", "plugin:worker"]
    );
    assert!(view.edges.is_empty());
    assert_eq!(view.family, GraphRelationFamily::Composition);
    assert_eq!(view.direction, GraphDirection::Upstream);

    let plugin_error = failure_graph(
        &failure,
        &GraphQuery {
            plugins: vec![String::from("component:worker")],
            ..GraphQuery::default()
        },
    )
    .expect_err("component cannot satisfy a plugin selector");
    let contributor_error = failure_graph(
        &failure,
        &GraphQuery {
            contributors: vec![String::from("missing")],
            ..GraphQuery::default()
        },
    )
    .expect_err("missing contributor is typed misuse");

    assert!(matches!(
        plugin_error,
        GraphQueryError::NotFound {
            kind: GraphSelectorKind::Plugin,
            ..
        }
    ));
    assert!(matches!(
        contributor_error,
        GraphQueryError::NotFound {
            kind: GraphSelectorKind::Contributor,
            ..
        }
    ));
}

fn failure(phase: Option<&str>, diagnostics: Vec<Diagnostic>) -> ProbeFailure {
    ProbeFailure {
        phase: phase.map(str::to_string),
        diagnostics,
    }
}

fn diagnostic<const N: usize>(code: &str, message: &str, resources: [&str; N]) -> Diagnostic {
    Diagnostic {
        code: code.to_string(),
        severity: DiagnosticSeverity::Error,
        message: message.to_string(),
        resources: resources.into_iter().map(str::to_string).collect(),
        sources: Vec::new(),
        fix: Some(String::from("repair it")),
    }
}

fn failure_graph(failure: &ProbeFailure, query: &GraphQuery) -> Result<GraphView, GraphQueryError> {
    query_failure_graph(
        Version::new(0, 20, 0),
        &DocumentIdentity {
            application: String::from("fixture"),
            ..DocumentIdentity::default()
        },
        failure,
        query,
    )
}

fn node_ids(view: &GraphView) -> BTreeSet<&str> {
    view.nodes.iter().map(|node| node.id.as_str()).collect()
}
