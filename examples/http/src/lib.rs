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

// Server-only for now: WebSocket/STOMP controllers and the DI-backed auth middleware need the
// server runtime; their generated clients are not wasm-ready yet (tracked as follow-up work).
#[cfg(not(target_family = "wasm"))]
pub mod auth;
#[cfg(not(target_family = "wasm"))]
pub mod stomp;
#[cfg(not(target_family = "wasm"))]
pub mod ws;
