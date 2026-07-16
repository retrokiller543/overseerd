//! OpenAPI document assembly for the axum protocol.
//!
//! The `#[dto]` and `#[handlers]` macros lower per-type and per-route metadata into two link-time
//! slices — [`OPENAPI_SCHEMAS`] (one entry per `#[dto]`, contributing its `utoipa::ToSchema`
//! component and its transitive dependencies) and [`OPENAPI_OPERATIONS`] (one entry per HTTP route,
//! contributing a `utoipa::openapi::path::Operation` built by `#[utoipa::path]`). [`build_openapi`]
//! folds both into a single [`utoipa::openapi::OpenApi`] at serve time; the model, schema
//! generation, and serialization are entirely utoipa's — this module only collects and joins.
//!
//! Mirrors the `CONTROLLERS` linkme pattern, so operations/schemas from independent `#[handlers]`
//! blocks and separate crates all compose with no central registration.

use utoipa::openapi::path::{HttpMethod, Operation};
use utoipa::openapi::schema::Schema;
use utoipa::openapi::{ComponentsBuilder, Info, OpenApi, OpenApiBuilder, Paths, RefOr, Server};

use crate::config::{OpenApiConfig, OpenApiUi};

/// One HTTP route's contribution to the document: its full path, the HTTP methods it serves, and
/// the utoipa [`Operation`]. Produced by a `#[handlers]`-generated closure over the route's
/// `#[utoipa::path]` type; the path is joined onto the controller base at call time.
pub type OperationEntry = fn() -> (String, Vec<HttpMethod>, Operation);

/// One `#[dto]` type's contribution to the document's components: it pushes its own
/// `(name, schema)` and those of its nested `#[dto]` fields, via `utoipa::ToSchema::schemas`.
pub type SchemaEntry = fn(&mut Vec<(String, RefOr<Schema>)>);

/// Every HTTP route registers one [`OperationEntry`] here (native + `openapi` only), mirroring the
/// `CONTROLLERS` slice. Folded by [`build_openapi`].
#[linkme::distributed_slice]
pub static OPENAPI_OPERATIONS: [OperationEntry];

/// Every `#[dto]` registers one [`SchemaEntry`] here, contributing its component schema.
#[linkme::distributed_slice]
pub static OPENAPI_SCHEMAS: [SchemaEntry];

/// Joins a controller `base` with a route's relative path the same way the router mounts it
/// (`Router::nest(base, ..)`): `("/users", "/{id}")` → `/users/{id}`; an empty or `"/"` base yields
/// the relative path; an empty relative path yields the base. The result always starts with `/`.
pub fn join_base(base: &str, relative: &str) -> String {
    let base = base.trim_end_matches('/');
    let relative = if relative == "/" { "" } else { relative };

    let joined = format!("{base}{relative}");

    if joined.is_empty() {
        String::from("/")
    } else {
        joined
    }
}

/// Folds the two link-time slices into a single OpenAPI document with the given `title`/`version`.
/// Operations sharing a path are merged into one path item (utoipa handles the per-method merge);
/// component schemas are deduped by name. A non-empty `base_path` becomes an OpenAPI `server` URL,
/// so documented operation paths stay relative to it (matching how the router nests under the
/// prefix). Pure — safe to call once at serve time.
pub fn build_openapi(title: &str, version: &str, base_path: &str) -> OpenApi {
    let mut paths = Paths::new();
    let mut schemas: Vec<(String, RefOr<Schema>)> = Vec::new();

    for entry in OPENAPI_OPERATIONS {
        let (path, methods, operation) = entry();

        paths.add_path_operation(&path, methods, operation);
    }

    for entry in OPENAPI_SCHEMAS {
        entry(&mut schemas);
    }

    let components = ComponentsBuilder::new().schemas_from_iter(schemas).build();
    let servers = normalize_prefix(base_path).map(|prefix| vec![Server::new(prefix)]);

    OpenApiBuilder::new()
        .info(Info::new(title, version))
        .paths(paths)
        .servers(servers)
        .components(Some(components))
        .build()
}

/// Normalizes a configured base path into an OpenAPI server prefix: `None` for an empty or `"/"`
/// path (routes at the root), otherwise the path with any trailing slash trimmed.
fn normalize_prefix(base_path: &str) -> Option<String> {
    let trimmed = base_path.trim_end_matches('/');

    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Mounts the OpenAPI surface onto `router` when [enabled](OpenApiConfig::enabled): the JSON
/// document at [`json_path`](OpenApiConfig::json_path), plus the configured UI (or none). The
/// document is built once here. A UI whose crate feature is not compiled logs a warning and is
/// skipped, leaving the JSON endpoint intact. Returns `router` unchanged when disabled.
///
/// `base_path` is the global prefix the router will be nested under; it is recorded as the
/// document's server URL. The routes mounted here are relative — they ride the same nesting.
pub fn mount(router: axum::Router, config: &OpenApiConfig, base_path: &str) -> axum::Router {
    if !config.enabled {
        return router;
    }

    let doc = build_openapi(&config.title, &config.version, base_path);

    // Swagger UI serves its own copy of the spec at `json_path`; every other choice (a UI that
    // inlines the doc, references our endpoint, or no UI at all) needs us to serve the JSON.
    let swagger_owns_json =
        cfg!(feature = "openapi-swagger-ui") && matches!(config.ui, OpenApiUi::Swagger);

    let router = if swagger_owns_json {
        router
    } else {
        serve_json(router, &config.json_path, doc.clone())
    };

    mount_ui(router, config, doc)
}

/// Adds a `GET {json_path}` route returning the serialized document.
fn serve_json(router: axum::Router, json_path: &str, doc: OpenApi) -> axum::Router {
    router.route(
        json_path,
        axum::routing::get(move || {
            let body = doc.clone();

            async move { axum::Json(body) }
        }),
    )
}

/// Mounts the configured UI onto `router`, if any. Each arm is gated on its crate feature; a UI
/// selected in config whose feature is absent logs a warning and serves JSON only.
fn mount_ui(router: axum::Router, config: &OpenApiConfig, doc: OpenApi) -> axum::Router {
    match config.ui {
        OpenApiUi::None => router,

        OpenApiUi::Swagger => {
            #[cfg(feature = "openapi-swagger-ui")]
            {
                // Pass the bare `ui_path`; the axum integration adds its own redirect + `{*rest}`
                // wildcard. Swagger serves the spec itself at `json_path`.
                let swagger = utoipa_swagger_ui::SwaggerUi::new(config.ui_path.clone())
                    .url(config.json_path.clone(), doc);

                router.merge(swagger)
            }

            #[cfg(not(feature = "openapi-swagger-ui"))]
            {
                warn_missing_ui("swagger", "openapi-swagger-ui");

                let _ = doc;

                router
            }
        }

        OpenApiUi::Redoc => {
            #[cfg(feature = "openapi-redoc")]
            {
                use utoipa_redoc::Servable;

                router.merge(utoipa_redoc::Redoc::with_url(config.ui_path.clone(), doc))
            }

            #[cfg(not(feature = "openapi-redoc"))]
            {
                warn_missing_ui("redoc", "openapi-redoc");

                let _ = doc;

                router
            }
        }

        OpenApiUi::Rapidoc => {
            #[cfg(feature = "openapi-rapidoc")]
            {
                // RapiDoc references the spec by URL (our `json_path` route), so the doc is unused.
                let _ = doc;
                let rapidoc = utoipa_rapidoc::RapiDoc::new(config.json_path.clone())
                    .path(config.ui_path.clone());

                router.merge(rapidoc)
            }

            #[cfg(not(feature = "openapi-rapidoc"))]
            {
                warn_missing_ui("rapidoc", "openapi-rapidoc");

                let _ = doc;

                router
            }
        }

        OpenApiUi::Scalar => {
            #[cfg(feature = "openapi-scalar")]
            {
                use utoipa_scalar::Servable;

                router.merge(utoipa_scalar::Scalar::with_url(config.ui_path.clone(), doc))
            }

            #[cfg(not(feature = "openapi-scalar"))]
            {
                warn_missing_ui("scalar", "openapi-scalar");

                let _ = doc;

                router
            }
        }
    }
}

/// Warns that a configured UI cannot be served because its crate feature is not compiled.
#[allow(dead_code)]
fn warn_missing_ui(ui: &str, feature: &str) {
    tracing::warn!(
        target: "overseerd::axum",
        ui,
        feature,
        "OpenAPI UI selected in config but its crate feature is not enabled; serving JSON only"
    );
}

#[cfg(test)]
mod tests;
