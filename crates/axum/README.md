# overseerd-axum

> The axum/HTTP protocol plugin for Overseerd: controllers, DI route extractors, WebSockets/STOMP, multipart, and a generated typed client.

Part of the [Overseerd](../../README.md) framework — the HTTP protocol layer over the protocol-agnostic `overseerd-app` core.

## Role

`overseerd-axum` is a [`ProtocolPlugin`] ([`AxumPlugin`]): it builds a real [`axum::Router`] from
`#[controller]` components and serves them over HTTP. It bridges the framework's dependency
injection into axum via the [`Inject`] extractor — a per-request scope layer threads an
`Arc<ScopeContainer>` through the request extensions, and `Inject<T>` resolves components from it, so
route handlers freely mix native axum extractors with DI. The bridge is deliberately thin and
one-directional: nothing in `overseerd-di` or `overseerd-core` knows axum exists.

Beyond plain REST it owns the HTTP-side extras that mirror the RPC protocol for the web: WebSocket
controllers ([`WebsocketController`], `#[controller(ws = ..)]`), a STOMP 1.2 pub/sub [`Broker`] with
a typed `#[topics]` contract, the `multipart/form-data` extractor, NDJSON/raw stream framing, and a
transport-generic generated client (`crate::client`) that also compiles to wasm/TypeScript.

## Usage

Most users depend on the [`overseerd`](../../README.md) facade, which re-exports this crate under
`overseerd::axum` — you rarely name it directly. Enable the `axum` feature; it pulls in the HTTP
protocol plus the controller macros. Declare `#[controller]`/`#[handlers]` with route attributes,
then build an [`App`] (an `App<AxumPlugin>`) and serve.

```rust
use overseerd::axum::prelude::*;
use overseerd::prelude::*;

#[dto]
pub struct Greeting {
    pub message: String,
}

#[controller(path = "/greet")]
pub struct GreetController;

#[handlers]
impl GreetController {
    /// `GET /greet/{who}` — mixes the axum `Path` extractor with DI-resolved state via `Inject`.
    #[get("/{who}")]
    async fn greet(&self, Path(who): Path<String>) -> Json<Greeting> {
        Json(Greeting { message: format!("Hello, {who}!") })
    }
}
```

The generated `GreetControllerClient` (under the `client` feature, with a `reqwest`/`hyper` backend)
issues typed calls from the same definition, and the crate compiles to `wasm32` as a browser client.

## Internal role

Native (server) builds depend on `overseerd-app`, `overseerd-core`, `overseerd-di`,
`overseerd-config`, and `overseerd-hooks`, plus `axum`/`tower`/`tokio`. It re-exports the agnostic
app surface (`App`, `AppBuilder`, `ProtocolPlugin`, `Serve`, …) so a standalone `overseerd-axum`
user has a single import. The macros come from the sibling `overseerd-axum-macros` crate (re-exported
here: `controller`, `handlers`, the route attrs, `dto`, `topics`), and codecs build on
`overseerd-transport`. The `overseerd` facade re-exports everything under `overseerd::axum`. Because
the generated client must build for wasm, the client/DTO/stream/STOMP-wire modules are wasm-safe,
while the server modules (controller, plugin, protocol, ws broker, extractors) are gated to non-wasm.

## Feature flags

| Feature | Effect |
|---|---|
| `client` | Compile the generated HTTP client SDK (envelopes + codec, transport-agnostic). Pick a backend to issue requests. |
| `reqwest` | reqwest client backend (rustls-tls, streaming; also the wasm `fetch` backend and browser `Connection`). Implies `client`. |
| `hyper` | hyper client backend. Implies `client`. |
| `tungstenite` | WebSocket/STOMP client transports (native + wasm). Implies `client` + `ws`. |
| `ws` | WebSocket controller support (`axum/ws`). |
| `stomp` | STOMP 1.2 pub/sub broker + `#[controller(ws = Stomp)]` handlers + typed client (`stomp-parser`). Implies `ws`. |
| `multipart` | The server-side `multipart/form-data` extractor (re-exported `axum::extract::Multipart`). |
| `uuid` | Pull in `uuid`; the `TopicParam for Uuid` impl activates under `all(stomp, uuid)`. |
| `wasm-ts` | Opt into the newer `tsify` `Ts<T>` wasm ABI for the browser client (needs unreleased `tsify`). |
| `yaml` / `watch` / `tracing-subscriber` | Forwarded config extras: YAML sources / reload on change / `init_tracing`. |
| `di-check` | Compile-time DI-graph validation (forwarded across app/di/config/macros). |
| `facade` | Set by the `overseerd` facade: root the macros' generated plugin types at `::overseerd::axum::*` instead of the standalone `::overseerd_axum::*`. |
