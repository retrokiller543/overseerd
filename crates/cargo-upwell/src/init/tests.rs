use std::path::Path;

use upwell_test_utils::TempFixture;

use super::{
    Catalog, CatalogError, InitError, InitRequest, TemplateKind, TemplateSelection, init_project,
};

#[test]
fn builtins_cover_each_supported_project_kind() {
    let error = Catalog::load(Some(Path::new("missing-catalog.toml")))
        .expect_err("explicit missing catalog is rejected");

    assert!(matches!(error, CatalogError::Read { .. }));

    let catalog = Catalog::builtins();

    assert_eq!(
        catalog.template_ids().collect::<Vec<_>>(),
        [
            "upwell/application",
            "upwell/application-workspace",
            "upwell/plugin",
            "upwell/protocol",
        ]
    );
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

    assert_eq!(selected.kind(), TemplateKind::Application);
    assert_eq!(selected.path(), Some(template.as_path()));
    assert_eq!(tool.id(), "team/axum");
    assert_eq!(tool.command(), "cargo-upwell-axum");
    assert_eq!(tool.package(), Some("cargo-upwell-axum"));
    assert_eq!(tool.protocols(), ["upwell/axum"]);
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
fn reserved_generator_values_cannot_be_overridden() {
    let fixture = TempFixture::new("cargo-upwell-reserved-value");
    let error = init_project(InitRequest {
        destination: fixture.child("generated"),
        name: Some(String::from("sample-app")),
        template: TemplateSelection::Catalog {
            template: None,
            catalog_path: None,
        },
        workspace: false,
        no_vcs: true,
        define: vec![String::from("upwell_version=9.9.9")],
        upwell_path: None,
    })
    .expect_err("reserved values are rejected");

    assert!(matches!(error, InitError::ReservedValue(name) if name == "upwell_version"));
}
