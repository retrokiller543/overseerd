use super::{StompConnect, StompPrincipal};

#[test]
fn connect_debug_redacts_the_passcode() {
    let connect = StompConnect::new(
        "example.test".to_owned(),
        Some("alice".to_owned()),
        Some("super-secret".to_owned()),
        vec![("authorization".to_owned(), "Bearer secret-token".to_owned())],
    );
    let debug = format!("{connect:?}");

    assert!(debug.contains("alice"));
    assert!(debug.contains("[REDACTED]"));
    assert!(debug.contains("authorization"));
    assert!(!debug.contains("super-secret"));
    assert!(!debug.contains("secret-token"));
}

#[test]
fn principal_carries_subject_and_attributes() {
    let principal = StompPrincipal::authenticated("alice").with_attribute("role", "admin");

    assert!(principal.is_authenticated());
    assert_eq!(principal.subject(), Some("alice"));
    assert_eq!(principal.attribute("role"), Some("admin"));
}
