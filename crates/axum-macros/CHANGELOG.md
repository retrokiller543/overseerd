# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.14.1](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-macros-v0.14.0...overseerd-axum-macros-v0.14.1) - 2026-07-17

### Added

- *(axum)* support opaque handler returns (Response / impl IntoResponse)

## [0.14.0](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-macros-v0.13.0...overseerd-axum-macros-v0.14.0) - 2026-07-17

### Added

- *(axum)* [**breaking**] DI-native STOMP auth + protocol-generic WS topics with per-message request/response ([#76](https://github.com/retrokiller543/overseerd/pull/76))

## [0.12.0](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-macros-v0.11.2...overseerd-axum-macros-v0.12.0) - 2026-07-08

### Added

- *(broker)* implement deliver method with backpressure support for concurrent subscribers

### Fixed

- *(review)* address PR #65 review findings
- *(review)* address PR #66 comments

### Other

- Added docs to all crates

## [0.11.2](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-macros-v0.11.1...overseerd-axum-macros-v0.11.2) - 2026-07-07

### Fixed

- *(axum)* never drop client methods for guard-consumed path params

## [0.11.0](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-macros-v0.10.0...overseerd-axum-macros-v0.11.0) - 2026-07-07

### Added

- *(axum)* drop custom guards from client codegen; add query/raw/multipart bodies + per-call & transport headers

## [0.10.0](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-macros-v0.9.1...overseerd-axum-macros-v0.10.0) - 2026-07-03

### Added

- *(axum)* STOMP subscribe/send wasm clients over a shared Connection
- Made the framework compile to wasm and be able to generate wasm rest clients for axum.

## [0.9.0](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-macros-v0.8.0...overseerd-axum-macros-v0.9.0) - 2026-07-02

### Added

- *(axum)* DI-backed middleware registration + RequestMeta request-scope seed

## [0.8.0](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-macros-v0.7.0...overseerd-axum-macros-v0.8.0) - 2026-07-02

### Added

- *(axum/ws/stomp)* codec-agnostic SEND path + graceful client DISCONNECT
- *(axum/ws/stomp)* templated topic destinations + cross-cutting uuid integration
- *(axum)* typed STOMP client codegen + end-to-end example
- *(axum/client/stomp)* hand-written STOMP client transport actor
- *(axum/ws/stomp)* #[topics] macro with pluggable codec
- *(axum/ws)* generalize WebsocketProtocol vocabulary for pluggable framing

### Fixed

- address PR #49 review findings

### Other

- *(axum/ws/stomp)* improve code formatting and readability across multiple files

## [0.7.0](https://github.com/retrokiller543/overseerd/compare/overseerd-axum-macros-v0.6.0...overseerd-axum-macros-v0.7.0) - 2026-06-30

### Other

- Feature/protocol agnostic ([#20](https://github.com/retrokiller543/overseerd/pull/20))
