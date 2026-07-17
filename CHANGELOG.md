# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.14.0](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.13.0...overseerd-v0.14.0) - 2026-07-17

### Added

- *(axum)* [**breaking**] DI-native STOMP auth + protocol-generic WS topics with per-message request/response ([#76](https://github.com/retrokiller543/overseerd/pull/76))

## [0.13.0](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.12.1...overseerd-v0.13.0) - 2026-07-12

### Added

- *(axum)* add config, STOMP auth, and client interceptors

### Fixed

- *(axum)* restore wasm client feature builds

## [0.12.0](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.11.2...overseerd-v0.12.0) - 2026-07-08

### Added

- *(jobs)* observable, controllable job scheduler
- *(jobs)* implement job scheduling with interval and cron support

### Other

- *(jobs)* expand the jobs example for the new capabilities

## [0.11.1](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.11.0...overseerd-v0.11.1) - 2026-07-07

### Added

- *(axum)* enhance multipart upload support with JS File/Blob integration

## [0.11.0](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.10.0...overseerd-v0.11.0) - 2026-07-07

### Added

- *(axum)* drop custom guards from client codegen; add query/raw/multipart bodies + per-call & transport headers

## [0.10.0](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.9.1...overseerd-v0.10.0) - 2026-07-03

### Added

- *(axum)* STOMP subscribe/send wasm clients over a shared Connection
- Made the framework compile to wasm and be able to generate wasm rest clients for axum.

### Other

- bring the README up to date
- address PR #55 review — daemon default-feature doc + Dto intent

## [0.9.1](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.9.0...overseerd-v0.9.1) - 2026-07-02

### Other

- added a way for rest handlers to publish stomp topics

## [0.9.0](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.8.0...overseerd-v0.9.0) - 2026-07-02

### Added

- *(axum)* DI-backed middleware registration + RequestMeta request-scope seed

## [0.8.0](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.7.0...overseerd-v0.8.0) - 2026-07-02

### Added

- *(axum/ws/stomp)* templated topic destinations + cross-cutting uuid integration
- *(axum)* typed STOMP client codegen + end-to-end example
- *(axum/ws)* generalize WebsocketProtocol vocabulary for pluggable framing

### Fixed

- address PR #49 review findings

### Other

- cargo fmt + require fmt/clippy gates before PR
- *(axum)* move test modules into their own files; document the rule
- *(stomp)* add docs/stomp.md tracking v1 scope and deferred features
- *(deps)* add arc-swap dependency to Cargo.lock
- add bug/security/performance hunter workflow

## [0.7.0](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.6.0...overseerd-v0.7.0) - 2026-06-30

### Other

- Feature/protocol agnostic ([#20](https://github.com/retrokiller543/overseerd/pull/20))

## [0.6.0](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.5.0...overseerd-v0.6.0) - 2026-06-26

### Other

- Config hot-reloading: Live/Dep, mutable Cfg, two-phase reload, hooks, and triggers ([#14](https://github.com/retrokiller543/overseerd/pull/14))

## [0.5.0](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.4.0...overseerd-v0.5.0) - 2026-06-25

### Other

- manager owns the config registry and seeds all defaults (fixes cross-path default references) ([#12](https://github.com/retrokiller543/overseerd/pull/12))

## [0.4.0](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.3.0...overseerd-v0.4.0) - 2026-06-25

### Other

- directory-namespace ergonomics + tagged-enum defaults ([#10](https://github.com/retrokiller543/overseerd/pull/10))

## [0.3.0](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.2.0...overseerd-v0.3.0) - 2026-06-25

### Added

- *(config)* select a default enum variant with #[default] ([#8](https://github.com/retrokiller543/overseerd/pull/8))

## [0.2.0](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.1.1...overseerd-v0.2.0) - 2026-06-25

### Other

- directory namespace, templated field defaults, enum support, app errors & unified logging ([#6](https://github.com/retrokiller543/overseerd/pull/6))

## [0.1.1](https://github.com/retrokiller543/overseerd/compare/overseerd-v0.1.0...overseerd-v0.1.1) - 2026-06-24

### Other

- release v0.1.0

## [0.1.0](https://github.com/retrokiller543/overseerd/releases/tag/overseerd-v0.1.0) - 2026-06-24

### Added

- *(macros)* factory = path, default_factory = false; sunset #[derive(Component)]
- *(core)* Factory construction traits + #[methods]; #[init] unified onto them
- *(core)* per-type factory slices; factories own their dependencies
- *(macros)* replace #[derive(ConfigProperties)] with #[config] attribute
- *(core)* per-service RPC slices; services own their RPC surface
- *(core)* type→descriptor connection and by-type registration
- *(core)* tower-based middleware, guards, and a global error handler

### Fixed

- fixed formating and fixed macro paths

### Other

- formatted code
- add PR checks workflow and wire release-plz token
- add crate metadata and internal dep versions for publishing
- *(todo)* record splitting core into smaller crates ([#23](https://github.com/retrokiller543/overseerd/pull/23))
- *(todo)* mark factory/manual macro unification ([#12](https://github.com/retrokiller543/overseerd/pull/12)) complete
- *(todo)* record object-safe DescriptorRuntime for runtime introspection
- *(todo)* mark builtins ([#10](https://github.com/retrokiller543/overseerd/pull/10)) and middleware ([#11](https://github.com/retrokiller543/overseerd/pull/11)) complete
- Merge branch 'feat/11-middleware' into feat/builtins-middleware
- added new task
- marked task as complete
- renamed the project to overseerd
- added commit rule to AGENTS.md
- kek
- Add .env.example and mise.toml for personal git identity configuration
- Add configuration system and application directories
- Optimize child scope allocation by reusing parent for empty scopes and conditionally seeding connection scope with PeerInfo
- Implement scoped dependencies with connection, request, and transient scopes
- Add support for streaming with custom codecs and enhance client call handling
- Add daemon! assembly macro, Wired graph check, and #[injectable] traits
- Add compile-time DI checking via Provide trait bounds (di-check feature)
- Add example daemon crate and extend build-time validation (v2)
- Add overseer-analyze: build-time DI validation (v1)
- Add dyn-trait providers, qualifiers, collections, and by-value injection
- Refactor dependency management to use `linkme` for component registration and enhance dynamic dependency handling
- Add client SDK generation support and enhance error handling
- Update TODO with additional considerations for client SDK generation
- Implement structured status codes for RPC error responses
- Reorder TODO by dependencies and value
- Mark streaming and Responder migration done in TODO
- Add streaming Echo service and interactive client to the example
- Infer streaming OperationKind from the #[rpc] signature
- Add streaming RPCs and Responder return-path migration
- Enhance DaemonBuilder with service registration and refactor component handling
- Remove unused HashMap import from example.rs
- Add #[component] macro for system-constructed components and enhance field injection
- Implement Component metadata trait and refactor component registration
- Add support for RPC groups and enhance component resolution logic
- Add support for manual component registration and validation in the daemon
- Add overseer-macros crate and implement service and rpc procedural macros
- Add typed parameter extraction and connection-scoped state management
- Cleaned up the protocol layer
- Added rough transport and protocol impl
- Add tokio dependency and implement component lifecycle management
- Fixed formatting
- Added basic core types and registry
- added core crate
- planned 001
- Added basic rust structure
- added docs
- init
- init
- Initial commit from Specify template

### Removed

- removed test commit
