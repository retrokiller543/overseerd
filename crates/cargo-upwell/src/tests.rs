use std::io;
use std::process::ExitCode;

use cargo_upwell::{Catalog, CommandExitCode};
use upwell_tooling_schema::{DocumentIdentity, Relationship, Resource, ToolingDocument};

use crate::{finish_output, inspect_component_payload, write_templates};

struct FailingWriter;

impl io::Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::WriteZero, "output closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn template_listing_reports_output_failures() {
    let result = write_templates(&Catalog::builtins(), &mut FailingWriter);
    let exit = finish_output(result, ExitCode::SUCCESS, "template catalog");

    assert_eq!(
        exit,
        ExitCode::from(CommandExitCode::OperationalFailure.code())
    );
}

#[test]
fn filtered_component_payload_contains_only_selected_resources() {
    let mut document = ToolingDocument {
        identity: DocumentIdentity {
            application: String::from("test"),
            ..DocumentIdentity::default()
        },
        protocol: String::from("test/protocol"),
        framework_version: env!("CARGO_PKG_VERSION").to_owned(),
        resources: ["resource:a", "resource:b"]
            .into_iter()
            .map(|id| Resource {
                id: id.to_owned(),
                name: id.to_owned(),
                ..Resource::default()
            })
            .collect(),
        ..ToolingDocument::default()
    };
    document.relationships.push(Relationship {
        from: String::from("resource:a"),
        to: String::from("resource:b"),
        ..Relationship::default()
    });
    let payload = inspect_component_payload(&document, &[String::from("resource:a")], false)
        .expect("projection serializes");
    let projected: ToolingDocument =
        serde_json::from_slice(&payload).expect("projection deserializes");

    assert_eq!(
        projected
            .resources
            .iter()
            .map(|resource| resource.id.as_str())
            .collect::<Vec<_>>(),
        ["resource:a"]
    );
    assert!(projected.relationships.is_empty());
    assert!(projected.cli.is_none());
}
