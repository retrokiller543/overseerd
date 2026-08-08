use upwell_test_utils::TempFixture;

use super::*;
use crate::renderer::component_descriptor;

#[test]
fn compiler_worker_entry() {
    if let Some(result) = run_component_compiler_worker() {
        result.expect("compiler worker succeeds");
    }
}
use crate::{RendererCapabilities, RendererDescriptor, RendererImplementation};

const VALID_COMPONENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/renderer/valid.component.wasm"
));
const FORBIDDEN_IMPORT_COMPONENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/renderer/forbidden-import.component.wasm"
));
const INFINITE_LOOP_COMPONENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/renderer/infinite-loop.component.wasm"
));
static TEST_SCHEMA: Version = Version::new(0, 20, 0);

#[test]
fn component_file_limit_is_enforced_before_wasmtime_compilation() {
    let fixture = TempFixture::new("cargo-upwell-renderer-size");
    let path = fixture.child("renderer.wasm");
    std::fs::write(&path, [0_u8; 9]).expect("fixture writes");
    let limits = ComponentLimits {
        component_bytes: 8,
        ..ComponentLimits::default()
    };

    assert!(matches!(
        read_bounded(&path, limits.component_bytes),
        Err(ComponentRenderError::ComponentTooLarge { limit: 8, .. })
    ));
}

#[test]
fn input_limit_is_enforced_before_component_loading() {
    let fixture = TempFixture::new("cargo-upwell-renderer-input");
    let component = fixture.child("missing.component.wasm");
    let renderer = test_renderer(component);
    let RendererImplementation::Component(renderer) = renderer.implementation() else {
        panic!("test renderer is a component")
    };
    let host = ComponentRendererHost::new(ComponentLimits {
        input_bytes: 1,
        ..ComponentLimits::default()
    })
    .expect("host builds");
    let schema = Version::new(0, 20, 0);
    let resources = Vec::new();

    assert!(matches!(
        host.render(
            renderer,
            ComponentRenderRequest {
                command: RendererCommand::Inspect,
                format: "custom",
                media_type: "text/plain",
                tooling_schema: &schema,
                resources: &resources,
                color: false,
                payload: b"too large",
            }
        ),
        Err(ComponentRenderError::InputTooLarge { limit: 1, .. })
    ));
}

#[test]
fn aggregate_request_limit_includes_resource_ids() {
    let fixture = TempFixture::new("cargo-upwell-renderer-resource-input");
    let descriptor = test_renderer(fixture.child("missing.component.wasm"));
    let RendererImplementation::Component(renderer) = descriptor.implementation() else {
        panic!("test renderer is a component")
    };
    let host = ComponentRendererHost::new(ComponentLimits {
        input_bytes: 12,
        ..ComponentLimits::default()
    })
    .expect("host builds");
    let resources = vec![String::from("resource:one")];

    assert!(matches!(
        host.render(renderer, request(&resources)),
        Err(ComponentRenderError::InputTooLarge { limit: 12, .. })
    ));
}

#[test]
fn incompatible_abi_is_rejected_before_component_loading() {
    let fixture = TempFixture::new("cargo-upwell-renderer-abi");
    let component = fixture.child("missing.component.wasm");
    let mut renderer = test_renderer(component);
    let RendererImplementation::Component(component) = &mut renderer.implementation else {
        panic!("test renderer is a component")
    };
    component.abi = "^2".parse().expect("requirement parses");
    let schema = Version::new(0, 20, 0);
    let resources = Vec::new();

    assert!(matches!(
        default_host().render(
            component,
            ComponentRenderRequest {
                command: RendererCommand::Inspect,
                format: "custom",
                media_type: "text/plain",
                tooling_schema: &schema,
                resources: &resources,
                color: false,
                payload: b"{}",
            }
        ),
        Err(ComponentRenderError::Abi { .. })
    ));
}

#[test]
fn core_wasm_module_is_not_accepted_as_a_component() {
    let fixture = TempFixture::new("cargo-upwell-renderer-core-module");
    let component = fixture.child("renderer.wasm");
    std::fs::write(&component, b"\0asm\x01\0\0\0").expect("core module fixture writes");
    let descriptor = test_renderer(component);
    let RendererImplementation::Component(renderer) = descriptor.implementation() else {
        panic!("test renderer is a component")
    };
    let schema = Version::new(0, 20, 0);
    let resources = Vec::new();

    assert!(matches!(
        default_host().render(
            renderer,
            ComponentRenderRequest {
                command: RendererCommand::Inspect,
                format: "custom",
                media_type: "text/plain",
                tooling_schema: &schema,
                resources: &resources,
                color: false,
                payload: b"{}",
            }
        ),
        Err(ComponentRenderError::InvalidComponent(_) | ComponentRenderError::CompilerFailed(_))
    ));
}

#[test]
fn valid_component_returns_deterministic_output_and_selected_resource_claims() {
    let fixture = component_fixture("valid", VALID_COMPONENT);
    let descriptor = test_renderer(fixture.child("renderer.component.wasm"));
    let RendererImplementation::Component(renderer) = descriptor.implementation() else {
        panic!("test renderer is a component")
    };
    let host = default_host();
    let schema = Version::new(0, 20, 0);
    let resources = vec![String::from("resource:one"), String::from("resource:two")];
    let request = ComponentRenderRequest {
        command: RendererCommand::Inspect,
        format: "custom",
        media_type: "text/plain",
        tooling_schema: &schema,
        resources: &resources,
        color: false,
        payload: br#"{"ignored":true}"#,
    };

    let first = host.render(renderer, request).expect("component renders");
    let second = host.render(renderer, request).expect("component rerenders");

    assert_eq!(first, b"fixture output");
    assert_eq!(second, first);
}

#[test]
fn component_imports_are_rejected_before_instantiation() {
    let fixture = component_fixture("forbidden-import", FORBIDDEN_IMPORT_COMPONENT);
    let descriptor = test_renderer(fixture.child("renderer.component.wasm"));
    let RendererImplementation::Component(renderer) = descriptor.implementation() else {
        panic!("test renderer is a component")
    };

    assert!(matches!(
        default_host().render(renderer, request(&[])),
        Err(ComponentRenderError::HostImport { name }) if name == "malicious:host/capability"
    ));
}

#[test]
fn unselected_component_resource_claim_is_rejected() {
    let fixture = component_fixture("resource-claim", VALID_COMPONENT);
    let descriptor = test_renderer(fixture.child("renderer.component.wasm"));
    let RendererImplementation::Component(renderer) = descriptor.implementation() else {
        panic!("test renderer is a component")
    };

    assert!(matches!(
        default_host().render(renderer, request(&[])),
        Err(ComponentRenderError::UnselectedResource)
    ));
}

#[test]
fn component_execution_fuel_is_enforced() {
    let fixture = component_fixture("out-of-fuel", VALID_COMPONENT);
    let descriptor = test_renderer(fixture.child("renderer.component.wasm"));
    let RendererImplementation::Component(renderer) = descriptor.implementation() else {
        panic!("test renderer is a component")
    };
    let host = ComponentRendererHost::new(ComponentLimits {
        fuel: 1,
        ..ComponentLimits::default()
    })
    .expect("host builds");

    assert!(matches!(
        host.render(renderer, request(&[String::from("resource:one")])),
        Err(ComponentRenderError::OutOfFuel)
    ));
}

#[test]
fn component_execution_deadline_interrupts_infinite_loop_before_fuel_exhaustion() {
    let fixture = component_fixture("deadline", INFINITE_LOOP_COMPONENT);
    let descriptor = test_renderer(fixture.child("renderer.component.wasm"));
    let RendererImplementation::Component(renderer) = descriptor.implementation() else {
        panic!("test renderer is a component")
    };
    let host = ComponentRendererHost::new(ComponentLimits {
        fuel: u64::MAX,
        deadline: EPOCH_TICK,
        ..ComponentLimits::default()
    })
    .expect("host builds");

    assert!(matches!(
        host.render(renderer, request(&[])),
        Err(ComponentRenderError::Deadline)
    ));
}

#[test]
fn component_output_limit_is_enforced_after_bounded_lifting() {
    let fixture = component_fixture("output-limit", VALID_COMPONENT);
    let descriptor = test_renderer(fixture.child("renderer.component.wasm"));
    let RendererImplementation::Component(renderer) = descriptor.implementation() else {
        panic!("test renderer is a component")
    };
    let host = ComponentRendererHost::new(ComponentLimits {
        output_bytes: 8,
        ..ComponentLimits::default()
    })
    .expect("host builds");

    assert!(matches!(
        host.render(renderer, request(&[String::from("resource:one")])),
        Err(ComponentRenderError::OutputTooLarge { size, limit: 8 }) if size > 14
    ));
}

#[test]
fn component_initial_memory_is_subject_to_store_limits() {
    let fixture = component_fixture("memory-limit", VALID_COMPONENT);
    let descriptor = test_renderer(fixture.child("renderer.component.wasm"));
    let RendererImplementation::Component(renderer) = descriptor.implementation() else {
        panic!("test renderer is a component")
    };
    let host = ComponentRendererHost::new(ComponentLimits {
        memory_bytes: 1024,
        ..ComponentLimits::default()
    })
    .expect("host builds");

    let result = host.render(renderer, request(&[String::from("resource:one")]));
    let Err(ComponentRenderError::Trap(error)) = result else {
        panic!("expected memory-limit trap, got {result:?}")
    };

    assert!(
        format!("{error:#}").contains("exceeds memory limits"),
        "unexpected trap: {error:#}"
    );
}

#[test]
fn component_instance_count_is_subject_to_store_limits() {
    let fixture = component_fixture("instance-limit", VALID_COMPONENT);
    let descriptor = test_renderer(fixture.child("renderer.component.wasm"));
    let RendererImplementation::Component(renderer) = descriptor.implementation() else {
        panic!("test renderer is a component")
    };
    let host = ComponentRendererHost::new(ComponentLimits {
        instances: 0,
        ..ComponentLimits::default()
    })
    .expect("host builds");

    let result = host.render(renderer, request(&[String::from("resource:one")]));
    let Err(ComponentRenderError::Trap(error)) = result else {
        panic!("expected instance-limit trap, got {result:?}")
    };

    assert!(
        format!("{error:#}").contains("instance count too high"),
        "unexpected trap: {error:#}"
    );
}

fn component_fixture(name: &str, component: &[u8]) -> TempFixture {
    let fixture = TempFixture::new(&format!("cargo-upwell-renderer-{name}"));

    std::fs::write(fixture.child("renderer.component.wasm"), component)
        .expect("component fixture writes");

    fixture
}

fn request(resources: &[String]) -> ComponentRenderRequest<'_> {
    ComponentRenderRequest {
        command: RendererCommand::Inspect,
        format: "custom",
        media_type: "text/plain",
        tooling_schema: &TEST_SCHEMA,
        resources,
        color: false,
        payload: b"{}",
    }
}

fn test_renderer(path: std::path::PathBuf) -> RendererDescriptor {
    component_descriptor(
        String::from("team/test"),
        path,
        vec![RendererCommand::Inspect],
        String::from("custom"),
        String::from("text/plain"),
        None,
        RendererCapabilities::default(),
        100,
        "^0.1".parse().expect("ABI parses"),
        "^0.20".parse().expect("schema parses"),
        true,
    )
}

fn default_host() -> ComponentRendererHost {
    ComponentRendererHost::new(ComponentLimits::default()).expect("host builds")
}
