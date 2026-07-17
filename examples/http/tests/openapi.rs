//! End-to-end OpenAPI generation: the `#[dto]`/`#[handlers]` macros lower schemas and operations
//! into the link-time slices, and `build_openapi` folds them into a document. These tests link the
//! example's controllers (so the slices are populated) and assert the resulting spec.

#![cfg(not(target_family = "wasm"))]

// Anchor the example library so the classic (macOS) linker actually processes its object files, and
// with them the `#[linkme]` OpenAPI/controller registrations the greeter contributes. A plain path
// reference can be dropped as dead code; `extern crate` forces the crate to be linked in.
extern crate overseerd_example_http;

use overseerd::axum::build_openapi;
use overseerd::axum::utoipa;

/// Builds the document once with the example's controllers linked in.
fn doc() -> utoipa::openapi::OpenApi {
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
    let doc = build_openapi("HTTP Example", "1.0.0", "/api");
    let servers = doc.servers.expect("a base path yields a server entry");

    assert_eq!(servers[0].url, "/api");
}

// An OpenAPI fixture controller defined **in the test binary**, not the example library — so the
// example app never links or serves it. It exercises two spec behaviours the plain greeter does not:
// a `Form` body (documented as `application/x-www-form-urlencoded`) and a handler that overrides the
// generated responses via `#[openapi(responses(..))]`.
mod fixture {
    use overseerd::axum::axum::extract::Form;
    use overseerd::axum::dto;
    use overseerd::axum::prelude::*;

    /// A form-encoded login submission.
    #[dto]
    pub struct LoginForm {
        pub user: String,
        pub password: String,
    }

    /// The login acknowledgement body.
    #[dto]
    pub struct LoginAck {
        pub user: String,
        pub ok: bool,
    }

    /// Documents OpenAPI-specific request/response shapes. Fieldless — it needs no state to feed the
    /// spec slices.
    #[controller(path = "/docs-demo")]
    pub struct DocsController;

    #[handlers]
    impl DocsController {
        /// `POST /docs-demo/login` — a form body, documented as `application/x-www-form-urlencoded`.
        #[post("/login")]
        async fn login(&self, Form(form): Form<LoginForm>) -> Json<LoginAck> {
            Json(LoginAck {
                user: form.user,
                ok: !form.password.is_empty(),
            })
        }

        /// `GET /docs-demo/teapot` — overrides the generated responses; the generated default `200`
        /// is dropped, so `utoipa::path` sees exactly one `responses` argument.
        #[get("/teapot")]
        #[openapi(responses(
            (status = 200, body = LoginAck, description = "brewed"),
            (status = 418, description = "I'm a teapot"),
        ))]
        async fn teapot(&self) -> Json<LoginAck> {
            Json(LoginAck {
                user: String::from("teapot"),
                ok: false,
            })
        }
    }
}

/// Builds the document with the in-test fixture controller linked in. The fixture lives in this test
/// binary, so its `#[linkme]` registrations are always part of the link — no anchor needed.
fn docs_doc() -> utoipa::openapi::OpenApi {
    build_openapi("HTTP Example", "1.0.0", "")
}

#[test]
fn form_request_body_is_documented_as_url_encoded() {
    let doc = docs_doc();
    let op = doc.paths.paths["/docs-demo/login"]
        .post
        .as_ref()
        .expect("POST /docs-demo/login is documented");
    let body = op.request_body.as_ref().expect("the form route has a body");

    // The `Form<T>` body must advertise the form media type, not utoipa's JSON default.
    assert!(
        body.content
            .contains_key("application/x-www-form-urlencoded"),
        "form body documents the urlencoded media type, got {:?}",
        body.content.keys().collect::<Vec<_>>()
    );
    assert!(
        !body.content.contains_key("application/json"),
        "form body must not be documented as JSON"
    );
}

#[test]
fn custom_responses_replace_the_generated_default() {
    let doc = docs_doc();
    let op = doc.paths.paths["/docs-demo/teapot"]
        .get
        .as_ref()
        .expect("GET /docs-demo/teapot is documented");

    // The custom `#[openapi(responses(..))]` set is present (that this even builds proves the
    // generated default was dropped rather than emitted as a second `responses` argument).
    assert!(
        op.responses.responses.contains_key("418"),
        "the custom 418 response is documented, got {:?}",
        op.responses.responses.keys().collect::<Vec<_>>()
    );
    assert!(
        op.responses.responses.contains_key("200"),
        "the custom 200 response is documented"
    );
}
