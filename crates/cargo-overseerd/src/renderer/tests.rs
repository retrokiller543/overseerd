use std::collections::BTreeMap;

use overseerd_test_utils::TempFixture;
use overseerd_tooling_schema::TOOLING_SCHEMA_VERSION;
use overseerd_tooling_schema::renderer::{RendererManifest, RendererView};
use overseerd_tooling_schema::{DocumentIdentity, Facet, Resource, ResourceKind, ToolingDocument};
use semver::VersionReq;

use super::{RendererManifestError, load_renderers, run_renderers};
use crate::CancellationToken;

#[test]
fn manifests_resolve_relative_executables_and_sort_deterministically() {
    let fixture = TempFixture::new("renderer-manifests");
    let executable = fixture.write("renderer", b"fixture");
    let second = fixture.write(
        "second.json",
        manifest("z-renderer", "plugin:acme/jobs", "renderer").as_bytes(),
    );
    let first = fixture.write(
        "first.json",
        manifest("a-renderer", "protocol:acme/http", "renderer").as_bytes(),
    );

    let renderers = load_renderers([second, first]).expect("manifests load");

    assert_eq!(renderers[0].manifest.id, "z-renderer");
    assert_eq!(renderers[1].manifest.id, "a-renderer");
    assert_eq!(renderers[0].executable, executable);
}

#[test]
fn duplicate_owner_claims_are_rejected_without_selecting_a_winner() {
    let fixture = TempFixture::new("renderer-duplicates");
    fixture.write("renderer", b"fixture");
    let first = fixture.write(
        "first.json",
        manifest("a-renderer", "protocol:acme/http", "renderer").as_bytes(),
    );
    let second = fixture.write(
        "second.json",
        manifest("b-renderer", "protocol:acme/http", "renderer").as_bytes(),
    );

    assert!(matches!(
        load_renderers([first, second]),
        Err(RendererManifestError::DuplicateOwner { .. })
    ));
}

#[test]
fn subprocess_renderer_produces_validated_presentation_only() {
    let executable = std::env::var_os("CARGO_BIN_EXE_tooling_renderer_fixture")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let current = std::env::current_exe().expect("test executable path is available");
            let debug = current
                .parent()
                .and_then(std::path::Path::parent)
                .expect("test executable is below target debug");

            debug.join(if cfg!(windows) {
                "tooling_renderer_fixture.exe"
            } else {
                "tooling_renderer_fixture"
            })
        });
    let fixture = TempFixture::new("renderer-process");
    let manifest_path = fixture.write(
        "renderer.json",
        manifest(
            "acme-http",
            "protocol:acme/http",
            executable.to_str().expect("executable path is UTF-8"),
        )
        .as_bytes(),
    );
    let renderers = load_renderers([manifest_path]).expect("renderer loads");
    let document = document();
    let run = run_renderers(
        &renderers,
        &document,
        RendererView::Inspect,
        document
            .resources
            .iter()
            .map(|resource| resource.id.clone())
            .collect::<Vec<_>>(),
        fixture.path(),
        &CancellationToken::default(),
    );

    assert!(run.diagnostics.is_empty());
    assert_eq!(
        run.presentation
            .resource("protocol:acme/http")
            .and_then(|presentation| presentation.label.as_deref()),
        Some("rendered HTTP")
    );
}

fn manifest(id: &str, owner: &str, executable: &str) -> String {
    let manifest = RendererManifest {
        schema: TOOLING_SCHEMA_VERSION,
        id: id.to_string(),
        owner: owner.to_string(),
        executable: executable.to_string(),
        document_schema: VersionReq::parse(&format!(
            "^{}.{}",
            TOOLING_SCHEMA_VERSION.major, TOOLING_SCHEMA_VERSION.minor
        ))
        .expect("requirement parses"),
        facets: BTreeMap::from([(format!("{owner}/tooling/summary"), vec![1])]),
    };

    serde_json::to_string(&manifest).expect("manifest serializes")
}

fn document() -> ToolingDocument {
    let mut owner = Resource {
        id: String::from("protocol:acme/http"),
        kind: ResourceKind::Protocol,
        name: String::from("HTTP"),
        ..Resource::default()
    };

    owner.facets.insert(
        String::from("protocol:acme/http/tooling/summary"),
        Facet {
            schema_version: 1,
            value: serde_json::json!({"route_count": 0}),
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

    document.resources.push(owner);
    document.canonicalize();

    document
}
