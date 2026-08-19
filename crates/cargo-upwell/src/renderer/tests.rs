use super::*;

#[test]
fn builtins_exactly_match_existing_command_formats_and_defaults() {
    let registry = RendererRegistry::new(built_in_descriptors()).expect("built-ins are valid");
    let expected = [
        (
            RendererCommand::Check,
            &["json", "terminal"][..],
            "terminal",
        ),
        (
            RendererCommand::Doctor,
            &["json", "terminal"][..],
            "terminal",
        ),
        (RendererCommand::Inspect, &["json", "text"][..], "text"),
        (
            RendererCommand::Export,
            &["document", "envelope"][..],
            "document",
        ),
        (
            RendererCommand::Graph,
            &["dot", "json", "mermaid", "text"][..],
            "text",
        ),
        (RendererCommand::Explain, &["json", "text"][..], "text"),
    ];

    for (command, formats, default) in expected {
        assert_eq!(
            registry
                .formats(command)
                .map(|renderer| renderer.format().id())
                .collect::<Vec<_>>(),
            formats
        );
        assert_eq!(registry.default_format(command), default);
    }
}

#[test]
fn registration_order_cannot_change_effective_resolution() {
    let low = test_component("team/low", 10);
    let high = test_component("team/high", 20);
    let first = RendererRegistry::new(
        built_in_descriptors()
            .into_iter()
            .chain([low.clone(), high.clone()]),
    )
    .expect("first registry builds");
    let second = RendererRegistry::new(built_in_descriptors().into_iter().chain([high, low]))
        .expect("second registry builds");

    assert_eq!(
        first
            .resolve(RendererCommand::Inspect, "custom")
            .expect("format resolves")
            .descriptor()
            .id(),
        "team/high"
    );
    assert_eq!(
        second
            .resolve(RendererCommand::Inspect, "custom")
            .expect("format resolves")
            .descriptor()
            .id(),
        "team/high"
    );
}

#[test]
fn lexical_id_breaks_equal_priority_ties() {
    let registry = RendererRegistry::new(built_in_descriptors().into_iter().chain([
        test_component("team/zeta", 10),
        test_component("team/alpha", 10),
    ]))
    .expect("registry builds");

    assert_eq!(
        registry
            .resolve(RendererCommand::Inspect, "custom")
            .expect("format resolves")
            .descriptor()
            .id(),
        "team/alpha"
    );
}

#[test]
fn formats_are_scoped_to_the_declared_commands() {
    let registry = RendererRegistry::new(
        built_in_descriptors()
            .into_iter()
            .chain([test_component("team/custom", 10)]),
    )
    .expect("registry builds");

    assert!(
        registry
            .resolve(RendererCommand::Inspect, "custom")
            .is_some()
    );
    assert!(registry.resolve(RendererCommand::Graph, "custom").is_none());
}

#[test]
fn machine_media_types_cannot_enable_color_or_paging() {
    let mut renderer = test_component("team/machine", 10);
    renderer.formats[0].media_type = String::from("application/json");
    renderer.formats[0].capabilities.color = true;

    assert!(matches!(
        RendererRegistry::new(built_in_descriptors().into_iter().chain([renderer])),
        Err(RendererRegistryError::MachinePresentationCapability {
            capability: "color",
            ..
        })
    ));
}

#[test]
fn capabilities_must_have_matching_command_line_options() {
    let mut renderer = test_component("team/file", 10);
    renderer.formats[0].capabilities.output_file = true;

    assert!(matches!(
        RendererRegistry::new(built_in_descriptors().into_iter().chain([renderer])),
        Err(RendererRegistryError::UnsupportedCommandCapability {
            command: RendererCommand::Inspect,
            capability: "output-file",
            ..
        })
    ));
}

#[test]
fn components_cannot_replace_canonical_builtin_format_ids() {
    let mut renderer = test_component("team/json", 10);
    renderer.formats[0].id = String::from("json");
    renderer.formats[0].media_type = String::from("application/json");
    renderer.formats[0].capabilities = RendererCapabilities::default();

    assert!(matches!(
        RendererRegistry::new(built_in_descriptors().into_iter().chain([renderer])),
        Err(RendererRegistryError::ReservedBuiltInFormat {
            command: RendererCommand::Inspect,
            format,
            ..
        }) if format == "json"
    ));
}

fn test_component(id: &str, priority: i32) -> RendererDescriptor {
    component_descriptor(
        id.to_owned(),
        PathBuf::from("renderer.wasm"),
        vec![RendererCommand::Inspect],
        String::from("custom"),
        String::from("text/plain"),
        Some(String::from("txt")),
        RendererCapabilities {
            color: false,
            pager: true,
            output_file: false,
        },
        priority,
        RENDERER_ABI_REQUIREMENT
            .parse()
            .expect("ABI requirement parses"),
        "^0.20".parse().expect("schema requirement parses"),
        true,
    )
}
