# overseerd-axum-macros

> The controller/route macros for the Overseerd axum/HTTP protocol: `#[controller]`, `#[handlers]`, the route attributes, `#[dto]`, and `#[topics]`.

Part of the [Overseerd](../../README.md) framework — the proc-macro companion to `overseerd-axum`, built on the shared `overseerd-macros-core` codegen.

## Role

This crate owns the axum/HTTP macro surface. It exists as a separate crate from the core
`overseerd-macros` because its generated code names plugin types under `::overseerd::axum::*` (the
HTTP protocol), which the protocol-agnostic core must not depend on. It provides:

- `#[controller]` — a **router component**: a `#[component]` (field-injected singleton) plus a
  controller header, its `{Controller}Routes` slice, and a `ControllerDescriptor`.
- `#[handlers]` — `MethodArgs<AxumHandlers>`: the shared base impl (`#[init]` + `#[hook]`) plus route
  registration, claiming each route-attributed method into the controller's routes slice. Multiple
  `#[handlers]` blocks for one controller merge without coordination.
- The route attributes — `#[get]`, `#[post]`, `#[put]`, `#[delete]`, `#[patch]`, `#[head]`,
  `#[options]`, the raw `#[route(METHOD, "/path")]`, and `#[message("dest")]` (WS handlers). These
  are inert markers consumed and stripped by `#[handlers]`.
- `#[dto]` — marks a type as HTTP wire data: derives `serde` (+ `tsify::Tsify` on wasm) and
  implements `Dto`, so a forgotten `#[dto]` is a clear error rather than a serde cascade.
- `#[topics]` — declares a STOMP topic-set enum, emitting an `impl Topic` (typed server publish) and
  a `{Enum}Client<C>` with one `subscribe_<variant>()` per topic, so the enum is the single source
  of truth for both sides.

## Usage

Most users depend on the [`overseerd`](../../README.md) facade, which re-exports these macros through
`overseerd-axum` under `overseerd::axum` — you never name this crate directly. Enable the `axum`
feature and use the attributes on your controllers.

```rust
use overseerd::axum::prelude::*;

#[dto]
pub struct Greeting { pub message: String }

#[controller(path = "/greet")]
pub struct GreetController;

#[handlers]
impl GreetController {
    #[get("/{who}")]
    async fn greet(&self, Path(who): Path<String>) -> Json<Greeting> {
        Json(Greeting { message: format!("Hello, {who}!") })
    }
}
```

## Internal role

A `proc-macro = true` crate built on `overseerd-macros-core` (which supplies `MethodArgs`, `Paths`,
`run`, `expand_component`, and the shared method codegen). It is a direct dependency of
`overseerd-axum`, which re-exports every macro at its crate root; the core macros (`app!`,
`#[component]`, …) come from `overseerd` instead. `axum_paths()` selects the generated plugin-type
root: `::overseerd::axum` under the `facade` feature, else the standalone `::overseerd_axum` — while
core vocabulary is always rooted at `::overseerd` either way.

## Feature flags

| Feature | Effect |
|---|---|
| `client` | Emit the generated HTTP client (forwarded to `overseerd-macros-core`). |
| `reqwest` | Pure codegen signal that the reqwest/fetch backend is available, so client codegen may emit the wasm `#[wasm_bindgen]` binding over `ReqwestClient`. Implies `client`. |
| `tungstenite` | Pure codegen signal that the WS transport is available, so STOMP `#[topics]`/`#[message]` codegen may emit the wasm binding over `StompClientTransport`. Implies `client`. |
| `wasm-ts` | Opt into the newer `tsify` `Ts<T>` wasm ABI: `#[dto]` derives plain `Tsify` and the client marshals via `Ts<T>` (needs unreleased `tsify`). |
| `di-check` | Emit compile-time DI assertions (forwarded to `overseerd-macros-core`). |
| `facade` | Root generated plugin types at `::overseerd::axum::*` (set by the `overseerd` facade) instead of the standalone `::overseerd_axum::*`. |
