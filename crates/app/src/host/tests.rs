use std::error::Error as _;

use super::{BootstrapContext, ExecutionMode, LifecyclePhase, PhaseError};

#[test]
fn bootstrap_context_stores_values_by_type() {
    let mut context = BootstrapContext::new(ExecutionMode::Tooling);

    assert!(context.mode().is_tooling());
    assert_eq!(context.insert(String::from("first")), None);
    assert_eq!(context.get::<String>().map(String::as_str), Some("first"));
    assert_eq!(
        context.insert(String::from("second")),
        Some(String::from("first"))
    );

    context
        .get_mut::<String>()
        .expect("string extension exists")
        .push_str(" value");

    assert_eq!(
        context.remove::<String>(),
        Some(String::from("second value"))
    );
    assert!(context.get::<String>().is_none());
}

#[test]
fn phase_error_preserves_phase_and_source() {
    let error = PhaseError::new(LifecyclePhase::BeforeBuild, std::io::Error::other("failed"));

    assert_eq!(error.phase(), LifecyclePhase::BeforeBuild);
    assert_eq!(
        error.source().map(ToString::to_string),
        Some(String::from("failed"))
    );
    assert_eq!(error.to_string(), "before_build phase failed: failed");
}
