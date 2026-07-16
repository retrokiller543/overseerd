use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::{build_openapi, join_base, mount, normalize_prefix};
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

    let router = mount(axum::Router::new(), &config, "");
    let (status, _) = get(router, "/openapi.json").await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "no route mounted when disabled"
    );
}

#[tokio::test]
async fn enabled_config_serves_json_document() {
    let router = mount(axum::Router::new(), &json_only_config(), "");
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

    let router = mount(axum::Router::new(), &config, "");

    assert_eq!(get(router.clone(), "/spec.json").await.0, StatusCode::OK);
    assert_eq!(get(router, "/openapi.json").await.0, StatusCode::NOT_FOUND);
}

#[cfg(feature = "openapi-swagger-ui")]
#[tokio::test]
async fn swagger_ui_mounts_and_owns_json() {
    let mut config = json_only_config();
    config.ui = OpenApiUi::Swagger;
    config.ui_path = String::from("/docs");

    let router = mount(axum::Router::new(), &config, "");

    // Swagger serves the spec itself at `json_path`, and the UI shell under `ui_path`.
    assert_eq!(get(router.clone(), "/openapi.json").await.0, StatusCode::OK);

    let (status, body) = get(router, "/docs/").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        body.to_lowercase().contains("swagger"),
        "serves the Swagger UI shell"
    );
}
