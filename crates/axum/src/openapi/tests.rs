use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::{build_openapi, join_base, mount, normalize_prefix, spec_url};
use crate::config::{OpenApiConfig, OpenApiUi};

/// Drives a `GET path` through `router` and returns `(status, body_string)`.
async fn get(router: axum::Router, path: &str) -> (StatusCode, String) {
    let response = router
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .expect("router responds");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body collected");

    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[test]
fn join_base_combines_prefix_and_relative() {
    assert_eq!(join_base("/users", "/{id}"), "/users/{id}");
    assert_eq!(join_base("/users/", "/{id}"), "/users/{id}");
    assert_eq!(join_base("/users", ""), "/users");
    assert_eq!(join_base("/users", "/"), "/users");
    assert_eq!(join_base("", "/health"), "/health");
    assert_eq!(join_base("/", "/health"), "/health");
    assert_eq!(join_base("", ""), "/");
}

#[test]
fn normalize_prefix_trims_and_empties() {
    assert_eq!(normalize_prefix(""), None);
    assert_eq!(normalize_prefix("/"), None);
    assert_eq!(normalize_prefix("/api"), Some(String::from("/api")));
    assert_eq!(normalize_prefix("/api/"), Some(String::from("/api")));
}

#[test]
fn spec_url_prefixes_json_path_with_base() {
    // No prefix: the UI fetches the bare json_path.
    assert_eq!(spec_url("", "/openapi.json"), "/openapi.json");

    // Under a base prefix, the UI must fetch the nested location, not the root one.
    assert_eq!(spec_url("/api", "/openapi.json"), "/api/openapi.json");
    assert_eq!(spec_url("/api/", "/spec.json"), "/api/spec.json");
}

#[test]
fn build_openapi_sets_info_and_no_server_without_prefix() {
    let doc = build_openapi("Test API", "1.2.3", "");

    assert_eq!(doc.info.title, "Test API");
    assert_eq!(doc.info.version, "1.2.3");
    assert!(doc.servers.is_none());
}

#[test]
fn build_openapi_records_base_path_as_server() {
    let doc = build_openapi("Test API", "1.2.3", "/api/");
    let servers = doc.servers.expect("a base path becomes a server entry");

    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].url, "/api");
}

/// A config with OpenAPI enabled, JSON only.
fn json_only_config() -> OpenApiConfig {
    OpenApiConfig {
        enabled: true,
        json_path: String::from("/openapi.json"),
        ui: OpenApiUi::None,
        ui_path: String::from("/docs"),
        title: String::from("Test"),
        version: String::from("1.0.0"),
    }
}

#[tokio::test]
async fn disabled_config_mounts_nothing() {
    let mut config = json_only_config();
    config.enabled = false;

    let router = mount(axum::Router::new(), &config, "").expect("mounts");
    let (status, _) = get(router, "/openapi.json").await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "no route mounted when disabled"
    );
}

// Redoc must be compiled for the overlap to actually materialize; validation now no-ops for an
// uncompiled UI (JSON-only fallback), so the rejection is only asserted when the UI is present.
#[cfg(feature = "openapi-redoc")]
#[test]
fn overlapping_json_and_ui_paths_are_rejected() {
    let mut config = json_only_config();
    config.ui = OpenApiUi::Redoc;
    config.ui_path = String::from("/docs");
    config.json_path = String::from("/docs");

    let error = mount(axum::Router::new(), &config, "")
        .expect_err("identical json_path and ui_path must be rejected");

    assert!(matches!(error, crate::Error::Config(_)), "got: {error}");

    // A json_path nested under the UI's wildcard is rejected too.
    config.json_path = String::from("/docs/openapi.json");

    assert!(
        mount(axum::Router::new(), &config, "").is_err(),
        "a json_path under ui_path must be rejected"
    );

    // Distinct paths are fine.
    config.json_path = String::from("/openapi.json");

    assert!(
        mount(axum::Router::new(), &config, "").is_ok(),
        "distinct paths mount cleanly"
    );
}

// When the selected UI's feature is absent, the UI is not mounted (JSON-only fallback), so
// overlapping paths must NOT be rejected. Runs only in a build without `openapi-scalar`.
#[cfg(not(feature = "openapi-scalar"))]
#[test]
fn overlap_is_ignored_when_the_selected_ui_is_not_compiled() {
    let mut config = json_only_config();
    config.ui = OpenApiUi::Scalar;
    config.ui_path = String::from("/docs");
    config.json_path = String::from("/docs");

    assert!(
        mount(axum::Router::new(), &config, "").is_ok(),
        "an uncompiled UI falls back to JSON only, so its path cannot collide"
    );
}

#[tokio::test]
async fn enabled_config_serves_json_document() {
    let router = mount(axum::Router::new(), &json_only_config(), "").expect("mounts");
    let (status, body) = get(router, "/openapi.json").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("\"openapi\""),
        "serves an OpenAPI document: {body}"
    );
    assert!(
        body.contains("\"title\":\"Test\""),
        "carries the configured info"
    );
}

#[tokio::test]
async fn json_served_at_custom_path() {
    let mut config = json_only_config();
    config.json_path = String::from("/spec.json");

    let router = mount(axum::Router::new(), &config, "").expect("mounts");

    assert_eq!(get(router.clone(), "/spec.json").await.0, StatusCode::OK);
    assert_eq!(get(router, "/openapi.json").await.0, StatusCode::NOT_FOUND);
}

#[cfg(feature = "openapi-swagger-ui")]
#[tokio::test]
async fn swagger_ui_mounts_and_owns_json() {
    let mut config = json_only_config();
    config.ui = OpenApiUi::Swagger;
    config.ui_path = String::from("/docs");

    let router = mount(axum::Router::new(), &config, "").expect("mounts");

    // Swagger serves the spec itself at `json_path`, and the UI shell under `ui_path`.
    assert_eq!(get(router.clone(), "/openapi.json").await.0, StatusCode::OK);

    let (status, body) = get(router, "/docs/").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        body.to_lowercase().contains("swagger"),
        "serves the Swagger UI shell"
    );
}

#[cfg(feature = "openapi-swagger-ui")]
#[tokio::test]
async fn swagger_under_base_path_serves_and_fetches_the_prefixed_spec() {
    let mut config = json_only_config();
    config.ui = OpenApiUi::Swagger;
    config.ui_path = String::from("/docs");

    // Mount with the normalized base prefix, then nest the whole router under it exactly as the
    // plugin does — so the test sees the real, browser-facing paths.
    let mounted = mount(axum::Router::new(), &config, "/api").expect("mounts");
    let router = axum::Router::new().nest("/api", mounted);

    // The spec route rode the nesting to `/api/openapi.json` — not double-prefixed to `/api/api/..`.
    assert_eq!(
        get(router.clone(), "/api/openapi.json").await.0,
        StatusCode::OK
    );
    assert_eq!(
        get(router.clone(), "/api/api/openapi.json").await.0,
        StatusCode::NOT_FOUND,
        "the spec must not be double-prefixed"
    );

    // The Swagger UI is told to fetch the prefixed URL, so it hits the served route.
    let (status, body) = get(router, "/api/docs/swagger-initializer.js").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("/api/openapi.json"),
        "the initializer fetches the base-prefixed spec URL, got: {body}"
    );
}
