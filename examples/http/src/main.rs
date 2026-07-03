//! The example HTTP daemon's **bootstrap** (binary-only). All controllers and components live in the
//! library ([`overseerd_example_http`]) so the crate doubles as a wasm browser client; this binary
//! only builds and serves the app.
//!
//! Run it, then exercise the routes:
//!
//! ```text
//! cargo run -p overseerd-example-http
//! curl localhost:3001/greet/world
//! curl -X POST localhost:3001/greet -H 'content-type: application/json' -d '"there"'
//! curl localhost:3001/greet/world/ticket
//!
//! # WebSocket (using websocat: https://github.com/vi/websocat)
//! echo '{"dest":"greet","id":1,"payload":{"who":"world"}}' | websocat ws://localhost:3001/ws
//! # → {"dest":"greet","id":1,"ok":{"message":"Hello, world!","count":1}}
//!
//! # Middleware (see the library's `auth` module):
//! curl localhost:3001/me/whoami -H 'authorization: Bearer alice'
//! # → {"name":"user:alice","same_instance":true}
//! ```

// Force the library into the link so its `#[controller]` registrations (link-time `linkme` slices
// that `auto_discover` folds in) reach the binary — a bin only links a dependency it references, and
// the controllers are self-registering, so nothing else names them. `extern crate` is the idiomatic
// linkage anchor and is warning-free (unlike `use … as _`); `linkme`'s `#[used]` statics do the rest.
extern crate overseerd_example_http;

// The binary is the native server bootstrap; on wasm the library is compiled as a browser client and
// there is no server to run, so the whole entry point is gated to non-wasm (with an inert wasm main
// so the bin target still compiles under `cargo build --target wasm32`).
#[cfg(not(target_family = "wasm"))]
mod server {
    use std::net::SocketAddr;

    use overseerd::axum::Stomp;
    use overseerd::axum::prelude::*;
    use overseerd::prelude::*;
    use overseerd_example_http::auth;

    pub async fn run() -> overseerd::axum::Result<()> {
        overseerd::builtins::init_tracing(&Default::default()).ok();

        // No `controllers:` listing: each `#[controller]` self-registers into the link-time slices
        // `auto_discover` folds in, so `app!` only needs the protocol. WebSockets are opt-in via
        // `register_ws`. `.layer(..)` takes a raw axum/tower layer directly (see `auth::log_requests`).
        let app = app! {
            name: "example-http",
            protocol: AxumPlugin,
        }
        .layer(overseerd::axum::axum::middleware::from_fn(
            auth::log_requests,
        ))
        .register_ws::<JsonWs>("/ws")
        .register_ws::<Stomp>("/ws/stomp")
        .build()
        .await?;

        println!("{app}");

        let addr = SocketAddr::from(([127, 0, 0, 1], 3001));
        println!("listening on http://{addr}");

        app.serve(addr).await
    }
}

#[cfg(not(target_family = "wasm"))]
#[tokio::main]
async fn main() -> overseerd::axum::Result<()> {
    server::run().await
}

#[cfg(target_family = "wasm")]
fn main() {}
