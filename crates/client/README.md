# overseerd-client

> The Overseerd protocol-agnostic client contract: capability traits and the shared error/streaming vocabulary that generated clients build on.

Part of the [Overseerd](../../README.md) framework — the wasm-compatible client SDK substrate, built on `overseerd-transport`.

## Role

This crate defines the client *contract* and implements no calls itself. It declares one trait per **capability** a protocol may support — [`Unary`], [`ServerStreaming`], [`ClientStreaming`], [`BidiStreaming`], over a shared [`Transport`] base — and a protocol (`overseerd-rpc`, a future HTTP binding, …) implements the subset it can. A protocol declares support by implementing a capability and refuses by simply not, so an unsupported call shape is a compile error rather than a runtime failure. It also owns the protocol-neutral vocabulary: [`ClientError`]/[`ErrorBody`] (carrying a protocol-defined opaque status), [`StreamArg`], and the target-conditional [`MaybeSend`]/[`MaybeSync`] markers that relax to no-ops on wasm. It assumes no serialization — messages are bound through the protocol's [`Encodes`]/[`Decodes`] impls, never a fixed `serde` bound.

## Usage

Most users depend on the [`overseerd`](../../README.md) facade, which re-exports this crate — you rarely name it directly. You never implement these traits; you meet the contract through a **generated** client (`FooClient<C>`) whose `C` is bound on exactly the capabilities its methods need, so the same generated code runs over any protocol providing them.

```rust
use overseerd::axum::client::ReqwestClient;

// The generated client delegates to the capability traits its transport `C` implements.
let client = GreetControllerClient::new(ReqwestClient::new("http://localhost:3000"));
let greeting = client.greet("world".into()).await?; // -> Greeting, fully typed
```

## Internal role

The protocol crates (`overseerd-rpc`, `overseerd-axum`) supply the concrete transport that implements these capability traits (`Unary::Request`/`Response`, the streaming `Responses` associated types) and pick the associated `Status` (RPC uses the packed `transport::StatusCode`; HTTP uses `http::StatusCode`). The client-codegen in `overseerd-macros` emits `FooClient<C>` types bounded on these traits. Because the crate depends only on `overseerd-transport` and `futures` and its unary path uses [`MaybeSend`]/[`MaybeSync`], it compiles to `wasm32-unknown-unknown` for the browser client.
