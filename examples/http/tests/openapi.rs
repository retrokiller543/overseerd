//! End-to-end OpenAPI generation: the `#[dto]`/`#[handlers]` macros lower schemas and operations
//! into the link-time slices, and `build_openapi` folds them into a document. These tests link the
//! example's controllers (so the slices are populated) and assert the resulting spec.

#![cfg(not(target_family = "wasm"))]

use overseerd::axum::build_openapi;
use overseerd::axum::utoipa;

/// Builds the document once with the example's controllers linked in.
fn doc() -> utoipa::openapi::OpenApi {
    // Reference a controller type so the linker keeps this crate's `CONTROLLERS`/OpenAPI slices.
    let _ = std::any::type_name::<overseerd_example_http::greet::GreetController>();

    build_openapi("HTTP Example", "1.0.0", "")
}

#[test]
fn greet_controller_routes_are_documented() {
    let doc = doc();
    let paths = &doc.paths.paths;

    // Base joined with each route's relative path.
    let greet_who = paths
        .get("/greet/{who}")
        .expect("GET /greet/{who} is documented");

    assert!(greet_who.get.is_some(), "the {{who}} route is a GET");

    let greet_root = paths.get("/greet").expect("/greet is documented");

    assert!(greet_root.get.is_some(), "GET /greet exists");
    assert!(greet_root.post.is_some(), "POST /greet exists");

    assert!(
        paths.contains_key("/greet/{who}/ticket"),
        "the ticketed route is documented"
    );
}

#[test]
fn path_parameter_is_typed_and_present() {
    let doc = doc();
    let op = doc.paths.paths["/greet/{who}"]
        .get
        .as_ref()
        .expect("GET /greet/{who}");
    let params = op.parameters.as_ref().expect("the route has a path param");

    assert!(
        params.iter().any(|p| p.name == "who"),
        "the `who` path parameter is documented, got {:?}",
        params.iter().map(|p| &p.name).collect::<Vec<_>>()
    );
}

#[test]
fn response_and_request_bodies_reference_dto_schemas() {
    let doc = doc();
    let components = doc.components.expect("components are present");

    // Every `#[dto]` used by the controller is a registered component schema.
    assert!(
        components.schemas.contains_key("GreetResponse"),
        "GreetResponse schema is registered, got {:?}",
        components.schemas.keys().collect::<Vec<_>>()
    );
    assert!(
        components.schemas.contains_key("TicketResponse"),
        "TicketResponse schema is registered"
    );
}

#[test]
fn base_path_becomes_a_server_entry() {
    let _ = std::any::type_name::<overseerd_example_http::greet::GreetController>();

    let doc = build_openapi("HTTP Example", "1.0.0", "/api");
    let servers = doc.servers.expect("a base path yields a server entry");

    assert_eq!(servers[0].url, "/api");
}
