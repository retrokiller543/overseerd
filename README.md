# Overseerd

Overseerd is a Rust framework for building long-running daemons and network services from strongly
typed components, services, and generated infrastructure — with compile-time dependency injection,
typed config, and typed clients generated from the same source of truth.

The goal is to make service development feel like writing ordinary Rust business logic while
Overseerd handles dependency wiring, endpoint registration, lifecycle, config, and the client SDK.

Unlike fully convention-driven frameworks, Overseerd never takes ownership of your entrypoint or
runtime. You keep control of process startup, runtime construction, and deployment while benefiting
from generated infrastructure and convention-assisted wiring.

> Overseerd is pre-1.0: the APIs below are implemented and exercised by the examples, but macro
> syntax and semantics may still evolve.

## Philosophy

> Boilerplate should be generated. Ownership should remain explicit.

Overseerd embraces code generation and metadata discovery to remove repetitive infrastructure while
keeping behavior inspectable, customizable, and overridable. It aims to sit between minimal runtime
libraries and fully managed application containers:

* More automation than low-level runtime libraries
* More explicitness than large convention-driven frameworks
* Strongly typed Rust APIs instead of stringly-typed configuration
* Convention-assisted, not convention-required

## Highlights

* **Compile-time dependency injection** — components and services are field-injected; a missing
  provider is a `cargo check` error (via the default `di-check` feature), not a runtime panic.
* **Two protocols, one core** — a native **RPC daemon** (`overseerd::daemon`) and an **axum/HTTP**
  plugin (`overseerd::axum`), both built on the same protocol-agnostic app/DI core. Run either, or
  both side by side.
* **Typed config** — `#[config]` types bound from a merged TOML/YAML tree, with `#[default]`s,
  `${VAR}`/`${@dir}` templating, and live reload hooks.
* **Generated typed clients** — every service/controller yields a transport-generic Rust client from
  its own definition; the HTTP client additionally generates a **wasm/TypeScript browser client**
  (REST + STOMP) with no hand-written bindings.
* **WebSockets & STOMP** — `#[controller(ws = ..)]` message handlers and a STOMP pub/sub broker with
  a typed `#[topics]` contract shared by server and client.
* **User-owned runtime** — Overseerd never requires ownership of `main`; you build the runtime, set
  up logging, and decide how to serve.

## Installation

```sh
cargo add overseerd
```

Pick what you need with features (all off by default except `di-check`):

| Feature | Enables |
|---|---|
| `daemon` | the native RPC protocol (`overseerd::daemon`) |
| `axum` | the HTTP protocol (`overseerd::axum`): `#[controller]`, routes, DI extractors |
| `ws` / `stomp` | WebSocket controllers / the STOMP pub/sub broker (imply `axum`) |
| `client` | generate the typed client SDK for the enabled protocol(s) |
| `reqwest` / `hyper` | HTTP client backends (pick one or both; `reqwest` also powers the wasm client) |
| `tungstenite` | the WebSocket/STOMP client transport (native + wasm) |
| `uuid` | `Uuid` support for templated STOMP topics |
| `yaml` | YAML config sources alongside TOML |
| `watch` | reload config on file change |
| `tracing-subscriber` | the `init_tracing` helper |
| `wasm-ts` | opt into the newer `tsify` `Ts<T>` wasm ABI for the browser client |
| `di-check` *(default)* | compile-time DI graph validation |

## An HTTP service

Put your controllers and components in the library (`lib.rs`) so the crate can also compile to a
browser client; keep `main.rs` as a thin bootstrap.

```rust
// lib.rs — the app surface
use overseerd::axum::prelude::*;
use overseerd::prelude::*;

/// A response body. `#[dto]` derives serde (+ TypeScript types on wasm) and marks it wire data.
#[dto]
pub struct Greeting {
    pub message: String,
}

/// A singleton dependency, field-injected into the controller.
#[component(by_value)]
#[derive(Clone)]
pub struct Greeter;

/// A controller mounted at `/greet`.
#[controller(path = "/greet")]
pub struct GreetController {
    greeter: Greeter,
}

#[handlers]
impl GreetController {
    /// `GET /greet/{who}` — mixes the axum `Path` extractor with the controller's own state.
    #[get("/{who}")]
    async fn greet(&self, Path(who): Path<String>) -> Json<Greeting> {
        Json(Greeting { message: format!("Hello, {who}!") })
    }
}
```

```rust
// main.rs — build and serve (you own the runtime)
use overseerd::axum::prelude::*;
use overseerd::prelude::*;

// Anchor the library so its self-registering `#[controller]`s are linked in.
extern crate my_service;

#[tokio::main]
async fn main() -> overseerd::axum::Result<()> {
    // Each `#[controller]` self-registers; `app!` only needs the protocol.
    let app = app! {
        name: "my-service",
        protocol: AxumPlugin,
    }
    .build()
    .await?;

    app.serve_configured().await
}
```

`AxumPlugin` always binds `[axum]`; every field has an environment-aware default, so the example
serves on `127.0.0.1:3000` even without a config file. Override listener and server-wide limits in
`application.toml` (or with the corresponding `AXUM_*` environment variables):

```toml
[axum]
bind = "0.0.0.0"
port = 8080
max_request_body_bytes = 2097152
request_timeout_ms = 30000
graceful_shutdown_timeout_ms = 30000
```

## An RPC daemon

```rust
use overseerd::daemon::{Inject, Payload, handlers, service};
use overseerd::{Cfg, Dep};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct NotifyRequest { pub message: String }

#[service(id = "notifications", version = "0.1")]
pub struct Notifications {
    #[config("app.greet")]
    config: Cfg<GreetConfig>,
}

#[handlers]
impl Notifications {
    #[rpc]
    async fn notify(
        &self,
        Payload(req): Payload<NotifyRequest>,
        Inject(db): Inject<Dep<DbConnection>>,
    ) -> NotifyResponse {
        // field-injected config + route-level DI, both resolved at compile time
        // ...
    }
}
```

```rust
use overseerd::daemon::prelude::*;

#[tokio::main]
async fn main() -> overseerd::daemon::Result<()> {
    let app = app! {
        name: "notifyd",
        protocol: RpcPlugin,
    }
    .build()
    .await?;

    // Serve over any transport — TCP, or a Unix socket on unix targets.
    app.serve(TcpTransport::bind("127.0.0.1:7000").await?).await
}
```

Both `#[service]`/`#[handlers]`/`#[rpc]` (RPC) and `#[controller]`/`#[handlers]`/`#[get]` (HTTP)
share the same base machinery: `#[component]` dependencies, `#[config]` values, `Inject<T>`
route-level DI, and `#[hook(..)]` lifecycle/config-reload hooks.

## Typed clients

Every service and controller generates a transport-generic Rust client from its own definition, so
the client can never drift from the server:

```rust
use overseerd::axum::client::ReqwestClient;

let client = GreetControllerClient::new(ReqwestClient::new("http://localhost:3000"));
let greeting = client.greet("world".into()).await?; // -> Greeting, fully typed
```

Both native HTTP backends store a generic interceptor value directly:

```rust
use overseerd::axum::client::{ClientInterceptor, ReqwestClient};
use overseerd::axum::http;

struct Hooks;

impl ClientInterceptor for Hooks {
    fn on_request(&self, request: &mut http::request::Parts) {
        request.headers.insert("authorization", "Bearer token".parse().unwrap());
    }

    fn on_response(&self, response: &mut http::response::Parts) {
        tracing::debug!(status = %response.status, "client response");
    }

    fn on_error<E>(&self, error: &overseerd::client::ClientError<http::StatusCode, E>) {
        tracing::error!(%error, "client call failed");
    }
}

let transport = ReqwestClient::new("http://localhost:3000").with_interceptor(Hooks);
let client = GreetControllerClient::new(transport);
```

A route the client can't express (e.g. a streamed body over HTTP/1.1) is simply absent from the
generated client — a protocol limit expressed as a compile error at the call site, never a wrong
call.

### Browser (wasm / TypeScript) clients

The same controller crate compiles to `wasm32-unknown-unknown` as a **browser client** — one
`wasm-pack build` at the wasm target, no separate crate, no hand-written `#[wasm_bindgen]`. The
server code is `cfg`-stripped; only the generated client survives, exported to JS under the
controller's own name with TypeScript types from your `#[dto]`s.

```js
import init, {
  Connection,
  GreetControllerClient,
  StompConnectOptions,
} from "./pkg";
await init();

// One shared Connection backs every client — one HTTP pool (+ cookies) and, for STOMP, one socket.
const conn = new Connection("http://localhost:3000");
conn.onRequest((request) => request.setHeader("authorization", "Bearer token"));
conn.onResponse((response) => console.debug("response", response.status));
conn.onError((error) => console.error(error.kind, error.message, error.status));

const greet = new GreetControllerClient(conn);

const greeting = await greet.greet("world"); // typed as Greeting in TS
```

STOMP works in the browser too — publish and subscribe over the shared connection, with typed
message callbacks generated from your `#[topics]` enum:

```js
const stompAuth = new StompConnectOptions();
stompAuth.setLogin("alice");
stompAuth.setPasscode("secret");
stompAuth.addHeader("tenant", "acme");
await conn.connectStompWithOptions("/ws/stomp", stompAuth); // ws:// derived from the base URL

const topics = new ChatTopicClient(conn);
const sub = await topics.subscribe_room("general", (msg /* : ChatMessage */) => {
  console.log(msg.text);
});

const chat = new ChatHandlerClient(conn);      // same socket
await chat.on_chat({ room: "general", sender: "me", text: "hi" });

sub.unsubscribe();
await conn.disconnectStomp();                  // closes the socket shared by every client
```

The `overseerd` DI/config core is wasm-safe; only the server-hosting pieces (socket transports, file
watching, the serve loops) are native-only, so a wasm build with any feature set just compiles.

## Design principles

### User-owned runtime

Overseerd never requires ownership of `main`. You remain free to configure logging before startup,
build custom Tokio runtimes, load environment variables, run startup validation, and integrate with
external tooling. The runtime helpers and convenience macros are optional.

### Convention-assisted discovery

Components, services, and `#[config]` types register themselves through generated metadata, but
everything discovered automatically can also be provided explicitly (e.g. binding a config type to a
path, or registering a component by value). Automatic registration is a convenience, not a rule.

### Magic must be inspectable

Generated infrastructure should never become invisible. The registry of components, their
dependency graph, services, RPC/route endpoints, and active transports is introspectable — printing
a built `app` shows the discovered surface.

### Metadata first

The macros primarily generate descriptors — component, service, RPC, config-binding, controller —
that runtime systems consume for execution, routing, validation, and client generation. This single
metadata model is what powers both the server and the generated (Rust and TypeScript) clients from
one source of truth.

## Non-goals

Overseerd is not intended to replace Tokio or existing observability ecosystems, hide all runtime
decisions, become a distributed-systems platform, or require framework ownership of application
startup or a specific deployment model.

## Examples

* `examples/daemon` — a complete RPC daemon: cross-module DI, merged config, and build-time
  DI-graph validation.
* `examples/http` — a complete HTTP + WebSocket + STOMP app whose library also compiles to a wasm
  browser client (`wasm-pack build examples/http --target web`).

## License

MIT.
