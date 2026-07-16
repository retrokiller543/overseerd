//! The example app's **surface**: every controller and component lives here, in the library, so it
//! is part of the reusable crate — not the binary. `main.rs` only wires up and serves the app.
//!
//! ## Recommended layout for an Overseerd axum project
//!
//! ```text
//! src/
//!   lib.rs    — controllers, components, config, DTOs (this file). The whole app surface.
//!   main.rs   — `#[tokio::main]` bootstrap: build the app and serve it. No business logic.
//! ```
//!
//! Keeping the controllers in the library (and out of `main.rs`) is what lets the **same crate**
//! compile to `wasm32-unknown-unknown` as a browser client: the framework macros gate the server
//! halves out on wasm, leaving the generated `{Controller}Client` — with no `main.rs` server code
//! polluting the wasm output. Build the client with `wasm-pack build examples/http --target web`.
//!
//! WebSocket / STOMP controllers and DI middleware are server-only for now, so those modules are
//! gated to non-wasm; the plain HTTP controller ([`greet`]) compiles for both.

// On wasm the server halves of the generated code are gated out, so the kept user types and their
// helpers are unused there — silence dead-code for the whole client-only build.
#![cfg_attr(target_family = "wasm", allow(dead_code))]

pub mod greet;

// The STOMP chat: its controllers' server halves are gated out on wasm by the macros, so the module
// compiles on both targets — a wasm client gets the generated `ChatTopicClient` (subscribe) and
// `ChatHandlerClient` (SEND) bound to the shared `Connection`, alongside the `ChatHistory` REST client.
pub mod stomp;

// Server-only: an OpenAPI documentation demo (a `Form` body + a custom-`responses` route). Not
// served — it only feeds the OpenAPI link-time slices for the spec-generation tests.
#[cfg(not(target_family = "wasm"))]
pub mod docs;

// Server-only for now: the JsonWs request/reply controller (no wasm ws transport yet) and the
// DI-backed auth middleware.
#[cfg(not(target_family = "wasm"))]
pub mod auth;
#[cfg(not(target_family = "wasm"))]
pub mod ws;
