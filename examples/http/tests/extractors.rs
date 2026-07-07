//! End-to-end test of client generation for the **full extractor surface**: a custom
//! `FromRequestParts` guard (dropped as server-only context, like `Inject`), a typed `Query<T>` and
//! the untyped `RawQuery`, and the three non-`Json`/`Form` bodies — raw `Bytes`, `RawForm`, and a
//! `Multipart` upload. Each handler is called through its macro-generated `{Controller}Client` over a
//! real server on an ephemeral port, so a route that classified wrong (no client method, or one that
//! silently drops an input) fails to compile or round-trips wrong here.

use overseerd::axum::Multipart;
use overseerd::axum::axum::Json;
use overseerd::axum::axum::body::Bytes;
use overseerd::axum::axum::extract::{FromRequestParts, Query, RawForm, RawQuery};
use overseerd::axum::axum::http::header::{HeaderMap, HeaderValue};
use overseerd::axum::axum::http::request::Parts;
use overseerd::axum::client::{Multipart as ClientMultipart, ReqwestClient};
use overseerd::axum::prelude::*;
use overseerd::prelude::*;
use tokio::net::TcpListener;

/// A custom `FromRequestParts` guard: the kind of auth/tenant extractor the client generator must
/// treat as server-only context and drop, so a guarded route still gets a client method. It reads an
/// optional header and never rejects, so the client (which does not send it) still round-trips.
struct ApiKey(String);

impl<S: Send + Sync> FromRequestParts<S> for ApiKey {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let key = parts
            .headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("anonymous")
            .to_string();

        Ok(ApiKey(key))
    }
}

/// A guard that consumes a **path segment** internally: it reads `Path::<String>` from the request
/// parts (the `{id}` hole) rather than the handler listing a `Path` arg. This is the regression case
/// from #61 — such a route must still get a client method, deriving the `id` param from the route
/// template. It never rejects, so the client round-trips.
struct Tenant(String);

impl<S: Send + Sync> FromRequestParts<S> for Tenant {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(id) = Path::<String>::from_request_parts(parts, state)
            .await
            .unwrap_or_else(|_| Path(String::new()));

        Ok(Tenant(id))
    }
}

/// A guard that consumes **two** path segments internally (`{id}` and `{child}`), exercising the
/// multi-hole `String`-fallback path — the client method must expose both holes as params.
struct TenantChild(String, String);

impl<S: Send + Sync> FromRequestParts<S> for TenantChild {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path((id, child)) = Path::<(String, String)>::from_request_parts(parts, state)
            .await
            .unwrap_or_else(|_| Path((String::new(), String::new())));

        Ok(TenantChild(id, child))
    }
}

/// A typed query — a flat `Dto` the client URL-encodes and the server decodes with `Query<T>`.
#[dto]
#[derive(PartialEq, Debug)]
struct Search {
    q: String,
    limit: u32,
}

/// The body a JSON handler returns alongside a query, to prove path + query + body compose.
#[dto]
#[derive(PartialEq, Debug)]
struct Combo {
    id: u64,
    q: String,
    sum: i64,
    key: String,
}

/// A JSON body used by the combined route.
#[dto]
struct Pair {
    a: i64,
    b: i64,
}

/// Exercises every extractor the client generator now understands beyond `Path`/`Json`.
#[controller(path = "/x")]
struct Extras {
    #[default]
    _unit: (),
}

#[handlers]
impl Extras {
    /// Custom guard only: the client method takes no arguments (the guard is dropped), and the
    /// server resolves the key from the request.
    #[get("/whoami")]
    async fn whoami(&self, key: ApiKey) -> Json<String> {
        Json(key.0)
    }

    /// Guard-consumed path param: the `{id}` hole is resolved *inside* the `Tenant` guard, so the
    /// handler lists no `Path` arg. The client method must still exist, deriving `id` from the route
    /// template (#61). Round-trips the id the guard read back out.
    #[get("/tenant/{id}/info")]
    async fn tenant_info(&self, tenant: Tenant) -> Json<String> {
        Json(tenant.0)
    }

    /// Two guard-consumed path params (`{id}`, `{child}`): both holes come from the template, and the
    /// client method exposes both as `String` params.
    #[get("/tenant/{id}/child/{child}")]
    async fn tenant_child(&self, pair: TenantChild) -> Json<String> {
        Json(format!("{}/{}", pair.0, pair.1))
    }

    /// Typed query: the client takes the raw `Search`, URL-encodes it, and the server decodes it.
    #[get("/search")]
    async fn search(&self, Query(query): Query<Search>) -> Json<Search> {
        Json(query)
    }

    /// Untyped query: the client takes an `Option<String>` appended verbatim.
    #[get("/raw-search")]
    async fn raw_search(&self, RawQuery(query): RawQuery) -> Json<String> {
        Json(query.unwrap_or_default())
    }

    /// Raw byte body: the client takes a `Vec<u8>` sent as `application/octet-stream`.
    #[post("/bytes")]
    async fn bytes_len(&self, body: Bytes) -> Json<usize> {
        Json(body.len())
    }

    /// Raw form body: the client takes a `Vec<u8>` sent under the form content type.
    #[post("/raw-form")]
    async fn raw_form(&self, RawForm(body): RawForm) -> Json<String> {
        Json(String::from_utf8(body.to_vec()).unwrap_or_default())
    }

    /// Multipart upload: the client builds a `Multipart` (text fields + files) and sends the encoded
    /// `multipart/form-data`; the server parses it back with axum's `Multipart` extractor.
    #[post("/upload")]
    async fn upload(&self, mut form: Multipart) -> Json<String> {
        let mut parts = Vec::new();

        while let Some(field) = form.next_field().await.expect("read multipart field") {
            let name = field.name().unwrap_or_default().to_string();
            let filename = field.file_name().map(str::to_string);
            let bytes = field.bytes().await.expect("read field bytes");

            parts.push(format!(
                "{name}:{}:{}",
                filename.unwrap_or_default(),
                bytes.len()
            ));
        }

        Json(parts.join(","))
    }

    /// Everything at once: a path param, a typed query, a JSON body, and the custom guard — the
    /// client method takes `(id, query, request)` and the guard is dropped.
    #[post("/combo/{id}")]
    async fn combo(
        &self,
        Path(id): Path<u64>,
        Query(query): Query<Search>,
        key: ApiKey,
        Json(body): Json<Pair>,
    ) -> Json<Combo> {
        Json(Combo {
            id,
            q: query.q,
            sum: body.a + body.b,
            key: key.0,
        })
    }
}

#[tokio::test]
async fn generated_client_covers_every_extractor() {
    let app = app! {
        name: "extractors-test",
        protocol: overseerd::axum::AxumPlugin,
    }
    .build()
    .await
    .expect("app builds");

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let shutdown = app.shutdown_handle();
    let server = tokio::spawn(async move { app.serve(listener).await });

    let client = ExtrasClient::new(ReqwestClient::new(format!("http://{addr}")));

    // Custom guard dropped: the clean method takes no arguments (the guard is server-only), and the
    // server resolves the (absent) key to its default.
    let who = client.whoami().await.expect("whoami call");
    assert_eq!(*who, "anonymous");

    // Guard-consumed path param (#61): the client method exists and takes `id` (a `String`, derived
    // from the route template) even though the handler has no `Path` arg — the guard reads `{id}`
    // internally. Before the fix this method was silently absent from the client.
    let info = client
        .tenant_info("acme".to_string())
        .await
        .expect("tenant_info call");
    assert_eq!(*info, "acme");

    // Two guard-consumed holes: both `id` and `child` are `String` params on the client method.
    let child = client
        .tenant_child("acme".to_string(), "widget".to_string())
        .await
        .expect("tenant_child call");
    assert_eq!(*child, "acme/widget");

    // Typed query: the client URL-encodes `Search`, the server decodes it, and it round-trips.
    let echoed = client
        .search(Search {
            q: "rust wasm".to_string(),
            limit: 25,
        })
        .await
        .expect("search call");
    assert_eq!(
        *echoed,
        Search {
            q: "rust wasm".to_string(),
            limit: 25
        }
    );

    // Untyped query: an `Option<String>` appended verbatim.
    let raw = client
        .raw_search(Some("a=1&b=2".to_string()))
        .await
        .expect("raw_search call");
    assert_eq!(*raw, "a=1&b=2");

    let empty = client.raw_search(None).await.expect("raw_search none");
    assert_eq!(*empty, "");

    // Raw byte body: `Vec<u8>` sent as octet-stream; the server reports its length.
    let len = client
        .bytes_len(vec![1u8, 2, 3, 4, 5])
        .await
        .expect("bytes call");
    assert_eq!(*len, 5);

    // Raw form body: `Vec<u8>` under the form content type; the server echoes it as text.
    let form = client
        .raw_form(b"name=ferris&lang=rust".to_vec())
        .await
        .expect("raw_form call");
    assert_eq!(*form, "name=ferris&lang=rust");

    // Multipart: build a text field + a file and send the encoded body; the server parses it back.
    let mut upload = ClientMultipart::new();
    upload.text("greeting".to_string(), "hello".to_string());
    upload.file(
        "doc".to_string(),
        "note.txt".to_string(),
        "text/plain".to_string(),
        b"multipart body bytes".to_vec(),
    );
    let summary = client.upload(upload).await.expect("upload call");
    assert_eq!(*summary, "greeting::5,doc:note.txt:20");

    // All four input kinds on one route: path + typed query + JSON body + dropped guard.
    let combo = client
        .combo(
            7,
            Search {
                q: "combined".to_string(),
                limit: 1,
            },
            Pair { a: 40, b: 2 },
        )
        .await
        .expect("combo call");
    assert_eq!(
        *combo,
        Combo {
            id: 7,
            q: "combined".to_string(),
            sum: 42,
            key: "anonymous".to_string(),
        }
    );

    // Per-call header: the parallel `_with_headers` method carries an extra `Option<HeaderMap>`. Pass
    // an `x-api-key` and the guard reads it and echoes it back.
    let mut per_call = HeaderMap::new();
    per_call.insert("x-api-key", HeaderValue::from_static("per-call-secret"));
    let who = client
        .whoami_with_headers(Some(per_call))
        .await
        .expect("whoami with per-call header");
    assert_eq!(*who, "per-call-secret");

    // Transport header provider: install a callback that stamps `x-api-key` on every request; the
    // next clean call picks it up.
    client.0.set_header_provider(|| {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("from-provider"));

        headers
    });
    let who = client.whoami().await.expect("whoami via provider");
    assert_eq!(*who, "from-provider");

    // Per-call headers still win over the provider on the same request.
    let mut per_call = HeaderMap::new();
    per_call.insert("x-api-key", HeaderValue::from_static("override"));
    let who = client
        .whoami_with_headers(Some(per_call))
        .await
        .expect("per-call overrides provider");
    assert_eq!(*who, "override");

    shutdown.shutdown();
    let _ = server.await;
}
