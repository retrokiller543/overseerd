use cargo_overseerd::{GraphDirection, GraphRelationFamily, GraphSource, GraphView};
use overseerd_tooling_schema::{
    Diagnostic, DiagnosticSeverity, DocumentIdentity, Relationship, RelationshipKind, Resource,
    ResourceKind,
};
use semver::Version;

use super::write_graph;
use crate::cli::GraphFormat;

#[test]
fn every_graph_format_is_deterministic_and_machine_formats_have_no_ansi() {
    let view = fixture();
    let mut reordered = view.clone();

    reordered.nodes.reverse();
    reordered.edges.reverse();
    reordered.diagnostics.reverse();

    for format in [
        GraphFormat::Text,
        GraphFormat::Mermaid,
        GraphFormat::Dot,
        GraphFormat::Json,
    ] {
        let first = render(&view, format, false);
        let second = render(&reordered, format, false);

        assert_eq!(first, second);

        if format != GraphFormat::Text {
            assert!(!first.contains("\u{1b}["));
        }
    }
}

#[test]
fn declarative_display_labels_text_graph_nodes() {
    let mut view = fixture();

    view.nodes[0].display = Some(overseerd_tooling_schema::ResourceDisplay {
        label: Some(String::from("rendered graph label")),
        ..Default::default()
    });

    let output = render(&view, GraphFormat::Text, false);

    assert!(output.contains("rendered graph label"));
    assert!(output.contains(&super::terminal_text(&view.nodes[0].id)));
}

#[test]
fn text_graph_marks_diagnostic_nodes_and_limits_color_to_heading_and_marker() {
    let output = render(&fixture(), GraphFormat::Text, true);

    assert!(output.contains("\u{1b}[1;36mApplication\u{1b}[0m"));
    assert!(output.contains("\u{1b}[1;31m!\u{1b}[0m"));
    assert!(output.contains("component      Worker"));
    assert!(output.contains("depends-on"));
}

#[test]
fn text_graph_escapes_controls_without_changing_line_structure() {
    let mut view = fixture();

    view.source.identity.application = String::from("app\u{1b}[31m\nnext\rline");
    view.diagnostics[0].message = String::from("bad\u{1b}[2J\nmessage\rreturn");

    let output = render(&view, GraphFormat::Text, false);

    assert!(!output.contains('\u{1b}'));
    assert!(!output.contains('\r'));
    assert!(output.contains(r"name: app\u{1b}[31m\nnext\rline"));
    assert!(output.contains(r"bad\u{1b}[2J\nmessage\rreturn"));
}

#[test]
fn mermaid_and_dot_escape_hostile_names_ids_and_labels() {
    let view = fixture();
    let mermaid = render(&view, GraphFormat::Mermaid, false);
    let dot = render(&view, GraphFormat::Dot, false);

    assert!(mermaid.starts_with("flowchart LR\n  n0[\""));
    assert!(mermaid.contains("#34;"));
    assert!(mermaid.contains("#92;"));
    assert!(mermaid.contains("\\n"));
    assert!(!mermaid.contains("node\"] -->"));
    assert!(dot.starts_with("digraph overseerd {\n  n0 [label=\""));
    assert!(dot.contains("\\\""));
    assert!(dot.contains("\\\\"));
    assert!(dot.ends_with("}\n"));
}

#[test]
fn incomplete_graph_formats_expose_status_phase_and_diagnostic_nodes() {
    let mut view = fixture();

    view.complete = false;
    view.failure_phase = Some(String::from("prepare\nphase\u{1b}[31m"));
    view.source.framework_version = None;
    view.source.protocol = None;
    view.edges.clear();

    let text = render(&view, GraphFormat::Text, false);
    let mermaid = render(&view, GraphFormat::Mermaid, false);
    let dot = render(&view, GraphFormat::Dot, false);
    let json = render(&view, GraphFormat::Json, false);
    let value: serde_json::Value = serde_json::from_str(&json).expect("graph JSON parses");

    assert!(text.contains("graph: diagnostic-only/incomplete"));
    assert!(text.contains(r"failure phase: prepare\nphase\u{1b}[31m"));
    assert!(text.contains("protocol: unavailable"));
    assert!(mermaid.contains("diagnostic-only/incomplete"));
    assert!(mermaid.starts_with("flowchart LR\n"));
    assert!(dot.contains("diagnostic-only/incomplete"));
    assert!(dot.ends_with("}\n"));
    assert_eq!(value["complete"], false);
    assert_eq!(value["failure_phase"], "prepare\nphase\u{1b}[31m");
    assert!(value["edges"].as_array().is_some_and(Vec::is_empty));
}

fn render(view: &GraphView, format: GraphFormat, color: bool) -> String {
    let mut output = Vec::new();

    write_graph(view, format, color, &mut output).expect("graph writes");

    String::from_utf8(output).expect("graph is UTF-8")
}

fn fixture() -> GraphView {
    GraphView {
        schema: Version::new(0, 20, 0),
        source: GraphSource {
            schema: Version::new(0, 20, 0),
            framework_version: Some(String::from("0.20.0")),
            identity: DocumentIdentity {
                application: String::from("fixture"),
                ..DocumentIdentity::default()
            },
            protocol: Some(String::from("fixture/protocol")),
        },
        complete: true,
        failure_phase: None,
        family: GraphRelationFamily::Dependencies,
        direction: GraphDirection::Both,
        roots: vec![String::from("component:hostile")],
        nodes: vec![
            Resource {
                id: String::from("component:hostile\"\\\nnode"),
                kind: ResourceKind::Component,
                name: String::from("Worker \"quoted\" \\ path\nnext"),
                ..Resource::default()
            },
            Resource {
                id: String::from("type:service"),
                kind: ResourceKind::Type,
                name: String::from("Service"),
                ..Resource::default()
            },
        ],
        edges: vec![Relationship {
            kind: RelationshipKind::DependsOn,
            from: String::from("component:hostile\"\\\nnode"),
            to: String::from("type:service"),
            labels: [(String::from("reason"), String::from("\"quoted\"\\\nvalue"))]
                .into_iter()
                .collect(),
        }],
        diagnostics: vec![Diagnostic {
            code: String::from("fixture/error"),
            severity: DiagnosticSeverity::Error,
            message: String::from("invalid worker"),
            resources: vec![String::from("component:hostile\"\\\nnode")],
            ..Diagnostic::default()
        }],
        cli_providers: Vec::new(),
    }
}
