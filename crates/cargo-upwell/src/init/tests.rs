use std::path::Path;

use upwell_test_utils::TempFixture;

use super::{
    Catalog, CatalogError, GitReference, InitError, InitRequest, TemplateSelection, TemplateSource,
    add_to_parent_workspace_with, init_project,
};
use crate::{RendererCommand, RendererImplementation};

#[test]
fn builtins_reference_tagged_canonical_repositories() {
    let error = Catalog::load(Some(Path::new("missing-catalog.toml")))
        .expect_err("explicit missing catalog is rejected");

    assert!(matches!(error, CatalogError::Read { .. }));

    let catalog = Catalog::builtins();

    assert_eq!(
        catalog
            .templates()
            .map(|template| template.id())
            .collect::<Vec<_>>(),
        ["upwell/application", "upwell/plugin", "upwell/protocol"]
    );
    assert!(catalog.templates().all(|template| matches!(
        template.source(),
        TemplateSource::Git {
            reference: Some(GitReference::Tag(tag)),
            ..
        } if tag == "v0.20.4"
    )));
}

#[test]
fn user_catalog_overrides_a_builtin_and_retains_typed_tools() {
    let fixture = TempFixture::new("cargo-upwell-catalog");
    let template = fixture.child("templates/application");
    let catalog_path = fixture.child("catalog.toml");

    std::fs::create_dir_all(&template).expect("template directory exists");
    fixture.write(
        "catalog.toml",
        r#"schema = "1"

[[entries]]
type = "template"
id = "upwell/application"
path = "templates/application"
description = "Team application"

[[entries]]
type = "tool"
id = "team/axum"
command = "cargo-upwell-axum"
package = "cargo-upwell-axum"
protocols = ["upwell/axum"]
"#,
    );

    let catalog = Catalog::load(Some(&catalog_path)).expect("user catalog loads");
    let selected = catalog
        .template("upwell/application")
        .expect("override is selected");
    let tool = catalog.tools().next().expect("typed tool is retained");

    assert_eq!(selected.source(), &TemplateSource::Local(template));
    assert_eq!(tool.id(), "team/axum");
    assert_eq!(tool.command(), "cargo-upwell-axum");
    assert_eq!(tool.package(), Some("cargo-upwell-axum"));
    assert_eq!(tool.protocols(), ["upwell/axum"]);
}

#[test]
fn user_catalog_retains_command_scoped_component_renderers() {
    let fixture = TempFixture::new("cargo-upwell-renderer-catalog");
    let catalog_path = fixture.child("catalog.toml");

    fixture.write(
        "catalog.toml",
        r#"schema = "1"

[[entries]]
type = "renderer"
id = "team/architecture"
component = "renderers/architecture.component.wasm"
commands = ["inspect", "graph"]
format = "architecture"
media-type = "text/plain"
extension = "txt"
pager = true
priority = 42
abi = "^0.1"
tooling-schema = "^0.20"
"#,
    );

    let catalog = Catalog::load(Some(&catalog_path)).expect("renderer catalog loads");
    let renderer = catalog.renderers().next().expect("renderer is retained");

    assert_eq!(renderer.id(), "team/architecture");
    assert_eq!(renderer.priority(), 42);
    assert_eq!(
        renderer
            .formats()
            .iter()
            .map(|format| (format.command(), format.id()))
            .collect::<Vec<_>>(),
        [
            (RendererCommand::Inspect, "architecture"),
            (RendererCommand::Graph, "architecture")
        ]
    );
    assert!(
        renderer
            .formats()
            .iter()
            .all(|format| format.capabilities().pager)
    );
    let RendererImplementation::Component(component) = renderer.implementation() else {
        panic!("catalog renderer uses a component");
    };
    assert_eq!(
        component.path(),
        fixture.child("renderers/architecture.component.wasm")
    );
    assert!(component.utf8());
}

#[test]
fn shipped_catalog_example_matches_the_catalog_schema() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("catalog.example.toml");

    Catalog::load(Some(&path)).expect("shipped example catalog parses");
}

#[test]
fn explicit_default_catalog_path_must_exist() {
    let Some(path) = super::default_catalog_path() else {
        return;
    };

    if path.exists() {
        return;
    }

    assert!(matches!(
        Catalog::load(Some(&path)),
        Err(CatalogError::Read { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound
    ));
}

#[test]
fn catalog_ids_follow_upwell_namespaced_id_syntax() {
    let fixture = TempFixture::new("cargo-upwell-catalog-invalid-id");

    for id in ["Team/template", "team/bad value", "team/$bad", "team/"] {
        fixture.write(
            "catalog.toml",
            format!(
                r#"schema = "1"

[[entries]]
type = "tool"
id = "{id}"
command = "cargo-team"
"#
            ),
        );

        assert!(matches!(
            Catalog::load(Some(&fixture.child("catalog.toml"))),
            Err(CatalogError::InvalidId(found)) if found == id
        ));
    }
}

#[test]
fn duplicate_user_ids_are_rejected_across_entry_kinds() {
    let fixture = TempFixture::new("cargo-upwell-catalog-duplicate");

    fixture.write(
        "catalog.toml",
        r#"schema = "1"

[[entries]]
type = "template"
id = "team/shared"
description = "Shared template"
path = "plugin"

[[entries]]
type = "tool"
id = "team/shared"
command = "cargo-team"
"#,
    );

    let error =
        Catalog::load(Some(&fixture.child("catalog.toml"))).expect_err("duplicate IDs are invalid");

    assert!(matches!(error, CatalogError::DuplicateId { id } if id == "team/shared"));
}

#[test]
fn tool_ids_cannot_collide_with_builtin_templates() {
    let fixture = TempFixture::new("cargo-upwell-catalog-builtin-collision");
    fixture.write(
        "catalog.toml",
        r#"schema = "1"

[[entries]]
type = "tool"
id = "upwell/application"
command = "cargo-upwell-application"
"#,
    );

    assert!(matches!(
        Catalog::load(Some(&fixture.child("catalog.toml"))),
        Err(CatalogError::DuplicateId { id }) if id == "upwell/application"
    ));
}

#[test]
fn git_templates_retain_one_explicit_reference() {
    let fixture = TempFixture::new("cargo-upwell-git-catalog");
    fixture.write(
        "catalog.toml",
        r#"schema = "1"

[[entries]]
type = "template"
id = "team/application"
description = "Team application"
git = "https://example.invalid/team/application.git"
revision = "0123456789abcdef"
"#,
    );

    let catalog =
        Catalog::load(Some(&fixture.child("catalog.toml"))).expect("Git catalog entry loads");
    let template = catalog
        .template("team/application")
        .expect("Git template is selected");

    assert_eq!(
        template.source(),
        &TemplateSource::Git {
            repository: String::from("https://example.invalid/team/application.git"),
            reference: Some(GitReference::Revision(String::from("0123456789abcdef"))),
        }
    );
}

#[test]
fn git_templates_reject_conflicting_references() {
    let fixture = TempFixture::new("cargo-upwell-git-reference-conflict");
    fixture.write(
        "catalog.toml",
        r#"schema = "1"

[[entries]]
type = "template"
id = "team/application"
description = "Team application"
git = "https://example.invalid/team/application.git"
branch = "main"
tag = "v1"
"#,
    );

    assert!(matches!(
        Catalog::load(Some(&fixture.child("catalog.toml"))),
        Err(CatalogError::ConflictingGitReferences { id }) if id == "team/application"
    ));
}

#[test]
fn templates_reject_empty_sources_and_git_references() {
    let fixture = TempFixture::new("cargo-upwell-empty-template-values");
    let entries = [
        ("path = \"\"", "path"),
        ("git = \" \"", "git"),
        (
            "git = \"https://example.invalid/template.git\"\nbranch = \" \"",
            "branch",
        ),
        (
            "git = \"https://example.invalid/template.git\"\ntag = \"\"",
            "tag",
        ),
        (
            "git = \"https://example.invalid/template.git\"\nrevision = \" \"",
            "revision",
        ),
    ];

    for (source, expected_field) in entries {
        fixture.write(
            "catalog.toml",
            format!(
                r#"schema = "1"

[[entries]]
type = "template"
id = "team/application"
description = "Team application"
{source}
"#
            ),
        );

        assert!(matches!(
            Catalog::load(Some(&fixture.child("catalog.toml"))),
            Err(CatalogError::EmptyTemplateValue { id, field })
                if id == "team/application" && field == expected_field
        ));
    }
}

#[test]
fn direct_local_template_is_expanded_by_cargo_generate() {
    let fixture = TempFixture::new("cargo-upwell-local-template");
    let template = fixture.child("template");
    let destination = fixture.child("generated");

    std::fs::create_dir_all(&template).expect("template directory exists");
    std::fs::write(
        template.join("message.txt.liquid"),
        "{{ project-name }}:{{ flavor }}",
    )
    .expect("template file is written");
    std::fs::write(
        template.join("cargo-generate.toml"),
        r#"[placeholders.flavor]
type = "string"
prompt = "Flavor"
"#,
    )
    .expect("template config is written");

    let result = init_project(InitRequest {
        destination: destination.clone(),
        name: Some(String::from("sample-app")),
        template: TemplateSelection::Local(template),
        workspace: false,
        no_vcs: true,
        define: vec![String::from("flavor=local")],
        upwell_path: None,
    })
    .expect("local template generates");

    assert_eq!(result.template, "local");
    assert_eq!(result.path, destination);
    assert_eq!(
        std::fs::read_to_string(result.path.join("message.txt"))
            .expect("generated file is readable"),
        "sample-app:local"
    );
}

#[test]
fn existing_destination_is_preserved() {
    let fixture = TempFixture::new("cargo-upwell-existing-destination");
    let template = fixture.child("template");
    let destination = fixture.child("generated");

    std::fs::create_dir_all(&template).expect("template directory exists");
    std::fs::write(template.join("generated.txt"), "generated").expect("template file is written");
    std::fs::create_dir(&destination).expect("destination is reserved by another actor");
    std::fs::write(destination.join("owner.txt"), "existing")
        .expect("existing destination marker is written");

    let error = init_project(InitRequest {
        destination: destination.clone(),
        name: Some(String::from("sample-app")),
        template: TemplateSelection::Local(template),
        workspace: false,
        no_vcs: true,
        define: Vec::new(),
        upwell_path: None,
    })
    .expect_err("existing destination is rejected");

    assert!(matches!(error, InitError::DestinationExists(path) if path == destination));
    assert_eq!(
        std::fs::read_to_string(destination.join("owner.txt"))
            .expect("existing marker remains readable"),
        "existing"
    );
    assert!(!destination.join("generated.txt").exists());
}

#[cfg(unix)]
#[test]
fn generated_destination_uses_normal_directory_permissions() {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = TempFixture::new("cargo-upwell-destination-permissions");
    let template = fixture.child("template");
    let destination = fixture.child("generated");
    let control = fixture.child("control");

    std::fs::create_dir_all(&template).expect("template directory exists");
    std::fs::write(template.join("generated.txt"), "generated").expect("template file is written");
    std::fs::create_dir(&control).expect("control directory is created through normal mkdir");

    init_project(InitRequest {
        destination: destination.clone(),
        name: Some(String::from("sample-app")),
        template: TemplateSelection::Local(template),
        workspace: false,
        no_vcs: true,
        define: Vec::new(),
        upwell_path: None,
    })
    .expect("local template generates");

    let destination_mode = std::fs::metadata(destination)
        .expect("generated destination metadata is readable")
        .permissions()
        .mode()
        & 0o777;
    let control_mode = std::fs::metadata(control)
        .expect("control directory metadata is readable")
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(destination_mode, control_mode);
}

#[test]
fn reserved_generator_values_cannot_be_overridden() {
    let fixture = TempFixture::new("cargo-upwell-reserved-value");
    let template = fixture.child("template");
    std::fs::create_dir_all(&template).expect("template directory exists");
    std::fs::write(template.join("file.txt"), "template").expect("template file is written");
    let error = init_project(InitRequest {
        destination: fixture.child("generated"),
        name: Some(String::from("sample-app")),
        template: TemplateSelection::Local(template),
        workspace: false,
        no_vcs: true,
        define: vec![String::from("upwell_version=9.9.9")],
        upwell_path: None,
    })
    .expect_err("reserved values are rejected");

    assert!(matches!(error, InitError::ReservedValue(name) if name == "upwell_version"));
}

#[test]
#[cfg(unix)]
fn workspace_registration_preserves_a_concurrent_manifest_edit() {
    let fixture = TempFixture::new("cargo-upwell-workspace-conflict");
    let project = fixture.child("generated");
    let manifest = fixture.child("Cargo.toml");
    let original = "[workspace]\nmembers = []\n";
    let concurrent = "[workspace]\nmembers = []\n\n[workspace.metadata.concurrent]\nvalue = true\n";

    std::fs::create_dir_all(&project).expect("project directory exists");
    std::fs::write(&manifest, original).expect("workspace manifest exists");

    let error = add_to_parent_workspace_with(
        &project,
        || {
            std::fs::write(&manifest, concurrent).expect("concurrent edit is written");
        },
        || {},
    )
    .expect_err("concurrent manifest edit is rejected");

    assert!(matches!(error, InitError::WorkspaceManifestConflict { .. }));
    assert_eq!(
        std::fs::read_to_string(&manifest).expect("manifest remains readable"),
        concurrent
    );
}

#[test]
#[cfg(unix)]
fn workspace_registration_preserves_an_atomic_manifest_replacement() {
    let fixture = TempFixture::new("cargo-upwell-workspace-replacement");
    let project = fixture.child("generated");
    let manifest = fixture.child("Cargo.toml");
    let replacement = fixture.child("Cargo.toml.editor");
    let original = "[workspace]\nmembers = []\n";
    let concurrent = "[workspace]\nmembers = []\n\n[workspace.metadata.editor]\nvalue = true\n";

    std::fs::create_dir_all(&project).expect("project directory exists");
    std::fs::write(&manifest, original).expect("workspace manifest exists");
    std::fs::write(&replacement, concurrent).expect("editor replacement exists");

    let error = add_to_parent_workspace_with(
        &project,
        || {
            std::fs::rename(&replacement, &manifest).expect("editor atomically replaces manifest");
        },
        || {},
    )
    .expect_err("atomic manifest replacement is rejected");

    assert!(matches!(error, InitError::WorkspaceManifestConflict { .. }));
    assert_eq!(
        std::fs::read_to_string(&manifest).expect("replacement remains readable"),
        concurrent
    );
}

#[test]
#[cfg(unix)]
fn workspace_registration_preserves_late_writes_and_recovery_bytes() {
    use std::io::{Seek as _, Write as _};

    let fixture = TempFixture::new("cargo-upwell-workspace-late-writer");
    let project = fixture.child("generated");
    let manifest = fixture.child("Cargo.toml");
    let original = "[workspace]\nmembers = []\n";
    let concurrent = "[workspace]\nmembers = []\n\n[workspace.metadata.late]\nvalue = true\n";

    std::fs::create_dir_all(&project).expect("project directory exists");
    std::fs::write(&manifest, original).expect("workspace manifest exists");
    let mut writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&manifest)
        .expect("late writer opens original inode");

    add_to_parent_workspace_with(
        &project,
        || {},
        || {
            writer.set_len(0).expect("late writer truncates old inode");
            writer.rewind().expect("late writer rewinds old inode");
            writer
                .write_all(concurrent.as_bytes())
                .expect("late writer updates old inode");
            writer.sync_all().expect("late writer syncs old inode");
        },
    )
    .expect("atomic publication succeeds while preserving the displaced inode");

    assert!(
        std::fs::read_to_string(&manifest)
            .expect("manifest remains readable")
            .contains("generated")
    );
    let recovery = std::fs::read_dir(fixture.path())
        .expect("fixture directory is readable")
        .filter_map(Result::ok)
        .find(|entry| entry.file_name().to_string_lossy().ends_with(".displaced"))
        .expect("late-writable displaced inode remains recoverable");
    assert_eq!(
        std::fs::read_to_string(recovery.path()).expect("recovery remains readable"),
        concurrent
    );
}

#[test]
#[cfg(unix)]
fn workspace_registration_preserves_manifest_permissions() {
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = TempFixture::new("cargo-upwell-workspace-permissions");
    let project = fixture.child("generated");
    let manifest = fixture.child("Cargo.toml");
    let original = "[workspace]\nmembers = []\n";

    std::fs::create_dir_all(&project).expect("project directory exists");
    std::fs::write(&manifest, original).expect("workspace manifest exists");
    std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o664))
        .expect("manifest permissions are configured");

    add_to_parent_workspace_with(&project, || {}, || {}).expect("workspace registration succeeds");

    assert_eq!(
        std::fs::metadata(&manifest)
            .expect("manifest metadata remains readable")
            .permissions()
            .mode()
            & 0o777,
        0o664
    );
}

#[test]
#[cfg(unix)]
fn workspace_registration_preserves_manifest_extended_attributes() {
    let fixture = TempFixture::new("cargo-upwell-workspace-xattrs");
    let project = fixture.child("generated");
    let manifest = fixture.child("Cargo.toml");
    let original = "[workspace]\nmembers = []\n";
    let attribute = "user.cargo-upwell-test";

    std::fs::create_dir_all(&project).expect("project directory exists");
    std::fs::write(&manifest, original).expect("workspace manifest exists");
    if let Err(error) = xattr::set(&manifest, attribute, b"preserved") {
        let code = error.raw_os_error();
        if code == Some(libc::ENOTSUP)
            || code == Some(libc::EOPNOTSUPP)
            || code == Some(libc::EPERM)
        {
            return;
        }
        panic!("test xattr is configured: {error}");
    }

    add_to_parent_workspace_with(&project, || {}, || {}).expect("workspace registration succeeds");

    assert_eq!(
        xattr::get(&manifest, attribute).expect("manifest xattrs remain readable"),
        Some(b"preserved".to_vec())
    );
}
