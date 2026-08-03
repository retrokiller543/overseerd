use syn::{Attribute, parse_quote};

use super::parse_route_attr;

#[test]
fn duplicate_modifiers_point_at_the_duplicate_key() {
    let attribute: Attribute = parse_quote!(
        #[get("/", returns = String, returns = Vec<u8>)]
    );
    let error = parse_route_attr(&attribute)
        .err()
        .expect("duplicate modifier fails");

    assert_eq!(error.to_string(), "duplicate `returns` modifier");
}

#[test]
fn duplicate_response_status_reports_both_declarations() {
    let attribute: Attribute = parse_quote!(
        #[get(
            "/",
            responses = [
                (status = 403, body = String),
                (status = 403, body = Vec<u8>),
            ]
        )]
    );
    let error = parse_route_attr(&attribute)
        .err()
        .expect("duplicate status fails");
    let rendered = error.into_compile_error().to_string();

    assert!(rendered.contains("duplicate response status 403"));
    assert!(rendered.contains("first declared here"));
}

#[test]
fn invalid_status_points_at_the_literal() {
    let attribute: Attribute = parse_quote!(
        #[get("/", responses = [(status = 999, body = String)])]
    );
    let error = parse_route_attr(&attribute)
        .err()
        .expect("invalid status fails");

    assert_eq!(
        error.to_string(),
        "response status must be between 100 and 599"
    );
}
