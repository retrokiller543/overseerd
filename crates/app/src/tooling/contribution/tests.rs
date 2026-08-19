use super::*;

#[test]
fn rejects_reserved_duplicate_and_unknown_owner_local_resources() {
    let mut reserved = ToolingContributions::new(String::from("plugin:third-party/test"));

    reserved.resource("component:spoof", "Spoof");

    assert!(matches!(
        reserved.finish(),
        Err(ToolingContributionError::ReservedId { .. })
    ));

    let mut duplicate = ToolingContributions::new(String::from("plugin:third-party/test"));

    duplicate.resource("route", "Route");
    duplicate.resource("route", "Route");

    assert!(matches!(
        duplicate.finish(),
        Err(ToolingContributionError::DuplicateResource { .. })
    ));

    let mut unknown = ToolingContributions::new(String::from("plugin:third-party/test"));

    unknown.relationship(
        ToolingRelationshipKind::Contains,
        ToolingEndpoint::Owner,
        ToolingEndpoint::Resource("missing"),
    );

    assert!(matches!(
        unknown.finish(),
        Err(ToolingContributionError::UnknownEndpoint { .. })
    ));
}

#[test]
fn owner_qualification_is_automatic_for_all_generic_endpoints() {
    let mut contributions = ToolingContributions::new(String::from("protocol:third-party/test"));

    contributions.display(ResourceDisplay {
        label: Some(String::from("Third-party protocol")),
        ..ResourceDisplay::default()
    });
    contributions.resource("transport", "Transport");
    contributions.relationship(
        ToolingRelationshipKind::Contains,
        ToolingEndpoint::Owner,
        ToolingEndpoint::Resource("transport"),
    );
    let contributions = contributions.finish().expect("metadata validates");

    assert_eq!(
        contributions.resources[0].id,
        "protocol:third-party/test/tooling/transport"
    );
    assert_eq!(
        contributions.relationships[0].from,
        "protocol:third-party/test"
    );
    assert_eq!(
        contributions.relationships[0].to,
        "protocol:third-party/test/tooling/transport"
    );
}

#[test]
fn protocol_display_is_required_and_resource_display_is_owner_local() {
    let missing = ToolingContributions::new(String::from("protocol:third-party/missing"));

    assert!(matches!(
        missing.finish(),
        Err(ToolingContributionError::MissingProtocolDisplay { .. })
    ));

    let mut contributions = ToolingContributions::new(String::from("protocol:third-party/test"));

    contributions.display(ResourceDisplay {
        label: Some(String::from("Third-party protocol")),
        ..ResourceDisplay::default()
    });
    contributions.resource("transport", "Transport");
    contributions.resource_display(
        "transport",
        ResourceDisplay {
            label: Some(String::from("HTTP transport")),
            summary: Some(String::from("Serves HTTP requests")),
            ..ResourceDisplay::default()
        },
    );

    let contributions = contributions.finish().expect("metadata validates");

    assert_eq!(
        contributions
            .owner_display
            .as_ref()
            .and_then(|display| display.label.as_deref()),
        Some("Third-party protocol")
    );
    assert_eq!(
        contributions.resources[0]
            .display
            .as_ref()
            .and_then(|display| display.label.as_deref()),
        Some("HTTP transport")
    );
}

#[test]
fn plugins_may_omit_display_but_invalid_declarations_fail() {
    ToolingContributions::new(String::from("plugin:third-party/test"))
        .finish()
        .expect("plugin display is optional");

    let mut unknown = ToolingContributions::new(String::from("plugin:third-party/test"));

    unknown.resource_display(
        "missing",
        ResourceDisplay {
            label: Some(String::from("Missing")),
            ..ResourceDisplay::default()
        },
    );

    assert!(matches!(
        unknown.finish(),
        Err(ToolingContributionError::UnknownEndpoint { .. })
    ));
}

#[test]
fn owner_qualification_is_injective_and_stable() {
    let owner = "plugin:third-party/a";

    assert_eq!(
        qualify(owner, "route/nested%id"),
        "plugin:third-party/a/tooling/route%2Fnested%25id"
    );
    assert_ne!(
        qualify("plugin:third-party/a", "b/route"),
        qualify("plugin:third-party/a/b", "route")
    );
    assert_ne!(
        qualify(owner, "route/nested"),
        qualify(owner, "route%2Fnested")
    );
}

#[test]
fn contribution_structure_is_validated_before_projection() {
    let mut invalid_owner = ToolingContributions::new(String::from("component:foreign"));
    let mut empty_name = ToolingContributions::new(String::from("plugin:third-party/test"));
    let mut invalid_facet = ToolingContributions::new(String::from("plugin:third-party/test"));

    invalid_owner.resource("local", "Local");
    empty_name.resource("local", " ");
    invalid_facet.facet("summary", 0, upwell_tooling_schema::JsonValue::Null);

    assert!(matches!(
        invalid_owner.finish(),
        Err(ToolingContributionError::InvalidOwner { .. })
    ));
    assert!(matches!(
        empty_name.finish(),
        Err(ToolingContributionError::EmptyResourceName { .. })
    ));
    assert!(matches!(
        invalid_facet.finish(),
        Err(ToolingContributionError::InvalidFacetVersion { .. })
    ));
}

#[test]
fn extension_relationships_reject_core_or_reversed_endpoint_shapes() {
    let mut reversed = ToolingContributions::new(String::from("plugin:third-party/test"));
    let mut dependency = ToolingContributions::new(String::from("plugin:third-party/test"));

    reversed.resource("local", "Local");
    reversed.relationship(
        ToolingRelationshipKind::Contains,
        ToolingEndpoint::Resource("local"),
        ToolingEndpoint::Owner,
    );
    dependency.resource("local", "Local");
    dependency.relationship(
        ToolingRelationshipKind::DependsOn,
        ToolingEndpoint::Owner,
        ToolingEndpoint::Resource("local"),
    );

    assert!(matches!(
        reversed.finish(),
        Err(ToolingContributionError::InvalidRelationshipEndpoints)
    ));
    assert!(matches!(
        dependency.finish(),
        Err(ToolingContributionError::InvalidRelationshipEndpoints)
    ));
}
