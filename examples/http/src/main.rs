//! The example HTTP daemon's **bootstrap** (binary-only). All controllers and components live in the
//! library ([`upwell_example_http`]) so the crate doubles as a wasm browser client; this binary
//! only builds and serves the app.
//!
//! Run it, then exercise the routes:
//!
//! ```text
//! cargo run -p upwell-example-http
//! curl localhost:3000/greet/world
//! curl -X POST localhost:3000/greet -H 'content-type: application/json' -d '"there"'
//! curl localhost:3000/greet/world/ticket
//!
//! # WebSocket (using websocat: https://github.com/vi/websocat)
//! echo '{"dest":"greet","id":1,"payload":{"who":"world"}}' | websocat ws://localhost:3000/ws
//! # → {"dest":"greet","id":1,"ok":{"message":"Hello, world!","count":1}}
//!
//! # Middleware (see the library's `auth` module):
//! curl localhost:3000/me/whoami -H 'authorization: Bearer alice'
//! # → {"name":"user:alice","same_instance":true}
//!
//! # OpenAPI (the crate is built with the `openapi-swagger-ui` feature). It is off by default;
//! # enable it (and pick a UI) via the config env vars, then browse the spec and Swagger UI:
//! AXUM_OPENAPI_ENABLED=true AXUM_OPENAPI_UI=swagger cargo run -p upwell-example-http
//! curl localhost:3000/openapi.json
//! # open http://localhost:3000/docs/  (Swagger UI)
//! ```

// Force the library into the link so its `#[controller]` registrations (link-time `linkme` slices
// that `auto_discover` folds in) reach the binary — a bin only links a dependency it references, and
// the controllers are self-registering, so nothing else names them. `extern crate` is the idiomatic
// linkage anchor and is warning-free (unlike `use … as _`); `linkme`'s `#[used]` statics do the rest.
extern crate upwell_example_http;

// The binary is the native server bootstrap; on wasm the library is compiled as a browser client and
// there is no server to run, so the whole entry point is gated to non-wasm (with an inert wasm main
// so the bin target still compiles under `cargo build --target wasm32`).
#[cfg(not(target_family = "wasm"))]
mod server {
    use upwell::axum::Stomp;
    use upwell::axum::prelude::*;
    use upwell::prelude::*;
    use upwell_example_http::auth;

    app! {
        /// Generated host for the native HTTP example server.
        app HttpApplication {
            name: "example-http",
            protocol: Axum,
            configure(_context, builder) {
                Ok::<_, std::convert::Infallible>(
                    builder
                        .layer(upwell::axum::axum::middleware::from_fn(
                            auth::log_requests,
                        ))
                        .register_ws::<JsonWs>("/ws")
                        .register_ws::<Stomp>("/ws/stomp"),
                )
            },
            serve(_context, app) {
                println!("{app}");

                let addr = app.protocol().configured_addr();
                println!("listening on http://{addr}");

                app.serve_configured().await
            },
        }
    }

    pub async fn run() -> Result<(), upwell::CliError> {
        HttpApplication::run().await
    }
}

#[cfg(not(target_family = "wasm"))]
#[tokio::main]
async fn main() -> Result<(), upwell::CliError> {
    server::run().await
}

#[cfg(target_family = "wasm")]
fn main() {}
