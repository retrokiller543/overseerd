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
fn duplicate_ids_are_rejected_even_when_owner_sorting_separates_them() {
    let fixture = TempFixture::new("renderer-duplicate-ids");
    fixture.write("renderer", b"fixture");
    let first = fixture.write(
        "first.json",
        manifest("duplicate", "plugin:acme/a", "renderer").as_bytes(),
    );
    let middle = fixture.write(
        "middle.json",
        manifest("unique", "plugin:acme/b", "renderer").as_bytes(),
    );
    let last = fixture.write(
        "last.json",
        manifest("duplicate", "plugin:acme/c", "renderer").as_bytes(),
    );

    assert!(matches!(
        load_renderers([first, middle, last]),
        Err(RendererManifestError::DuplicateId { .. })
    ));
}

#[test]
#[cfg(unix)]
fn subprocess_renderer_produces_validated_presentation_only() {
    let fixture = TempFixture::new("renderer-process");
    let executable = fixture.write("renderer.sh", "#!/bin/sh\nexit 1\n");

    {
        use std::os::unix::fs::PermissionsExt as _;

        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
            .expect("renderer fixture is executable");
    }

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
    let request = overseerd_tooling_schema::renderer::RendererRequest::new(
        &renderers[0].manifest,
        &document,
        RendererView::Inspect,
        document
            .resources
            .iter()
            .map(|resource| resource.id.clone()),
    )
    .expect("fixture request validates");
    let response = overseerd_tooling_schema::renderer::RendererResponse {
        schema: TOOLING_SCHEMA_VERSION,
        renderer: request.renderer.clone(),
        owner: request.owner.clone(),
        presentation: overseerd_tooling_schema::renderer::RendererPresentation {
            resources: vec![overseerd_tooling_schema::renderer::ResourcePresentation {
                resource: String::from("protocol:acme/http"),
                label: Some(String::from("rendered HTTP")),
                ..Default::default()
            }],
        },
    };
    let response_json = response
        .to_json(&request)
        .expect("fixture response validates");
    let response_path = fixture.write("expected-response.json", response_json);
    let executable = fixture.write(
        "renderer.sh",
        format!(
            "#!/bin/sh\ncp \"{}\" \"$OVERSEERD_TOOLING_RENDERER_RESPONSE\"\n",
            response_path.display()
        ),
    );

    {
        use std::os::unix::fs::PermissionsExt as _;

        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
            .expect("renderer fixture is executable");
    }

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
