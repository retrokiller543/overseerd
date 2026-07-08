# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.12.0](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-v0.11.2...overseerd-axum-v0.12.0) - 2026-07-08

### Added

- *(broker)* implement deliver method with backpressure support for concurrent subscribers
- *(connection)* add is_stomp_connected method for STOMP connection status

### Fixed

- *(review)* address PR #65 review findings
- *(review)* address PR #66 comments

### Other

- Added docs to all crates

## [0.11.1](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-v0.11.0...overseerd-axum-v0.11.1) - 2026-07-07

### Added

- *(axum)* enhance multipart upload support with JS File/Blob integration

## [0.11.0](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-v0.10.0...overseerd-axum-v0.11.0) - 2026-07-07

### Added

- *(axum)* drop custom guards from client codegen; add query/raw/multipart bodies + per-call & transport headers

## [0.10.0](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-v0.9.1...overseerd-axum-v0.10.0) - 2026-07-03

### Added

- *(axum)* STOMP subscribe/send wasm clients over a shared Connection
- Made the framework compile to wasm and be able to generate wasm rest clients for axum.

### Other

- address PR #55 review — daemon default-feature doc + Dto intent

## [0.9.1](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-v0.9.0...overseerd-axum-v0.9.1) - 2026-07-02

### Other

- added a way for rest handlers to publish stomp topics

## [0.9.0](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-v0.8.0...overseerd-axum-v0.9.0) - 2026-07-02

### Added

- implement Provide trait for RequestMeta in DI context
- *(axum)* DI-backed middleware registration + RequestMeta request-scope seed

## [0.8.0](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-v0.7.0...overseerd-axum-v0.8.0) - 2026-07-02

### Added

- *(axum/ws/stomp)* codec-agnostic SEND path + graceful client DISCONNECT
- *(axum/ws/stomp)* templated topic destinations + cross-cutting uuid integration
- *(axum/ws)* per-protocol Options + register_ws_with; drop register_stomp need
- *(example)* functional STOMP chat controller + fix server heart-beat handling
- *(axum)* typed STOMP client codegen + end-to-end example
- *(axum/client/stomp)* hand-written STOMP client transport actor
- *(axum/ws/stomp)* #[topics] macro with pluggable codec
- *(axum/ws/stomp)* server serve loop, DI seeding, and publish surface
- *(axum/ws/stomp)* broker, body types, and error enum
- *(axum/ws)* generalize WebsocketProtocol vocabulary for pluggable framing

### Fixed

- address PR #49 review findings
- *(axum/ws/stomp)* tolerate a CONNECT without a host header
- send a proper WS close frame on connection-scope-open failure
- Send error message before closing the connection if we fail to create teh connection scope

### Other

- cargo fmt + require fmt/clippy gates before PR
- *(axum)* move test modules into their own files; document the rule
- *(stomp)* add docs/stomp.md tracking v1 scope and deferred features
- *(axum/ws/stomp)* improve code formatting and readability across multiple files

## [0.7.0](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-v0.6.0...overseerd-axum-v0.7.0) - 2026-06-30

### Other

- Feature/protocol agnostic ([#20](https://github.com/retrokiller543/overseerd/pull/20))
