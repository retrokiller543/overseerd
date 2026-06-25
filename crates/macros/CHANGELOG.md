# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/retrokiller543/overseerd/compare/overseerd-macros-v0.1.1...overseerd-macros-v0.2.0) - 2026-06-25

### Other

- directory namespace, templated field defaults, enum support, app errors & unified logging ([#6](https://github.com/retrokiller543/overseerd/pull/6))

## [0.1.1](https://github.com/retrokiller543/overseerd/compare/overseerd-macros-v0.1.0...overseerd-macros-v0.1.1) - 2026-06-24

### Other

- release v0.1.0

## [0.1.0](https://github.com/retrokiller543/overseerd/releases/tag/overseerd-macros-v0.1.0) - 2026-06-24

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
- add crate metadata and internal dep versions for publishing
- renamed the project to overseerd
- Add configuration system and application directories
- Implement scoped dependencies with connection, request, and transient scopes
- Add support for streaming with custom codecs and enhance client call handling
- Add daemon! assembly macro, Wired graph check, and #[injectable] traits
- Add compile-time DI checking via Provide trait bounds (di-check feature)
- Add dyn-trait providers, qualifiers, collections, and by-value injection
- Refactor dependency management to use `linkme` for component registration and enhance dynamic dependency handling
- Add client SDK generation support and enhance error handling
- Implement structured status codes for RPC error responses
- Infer streaming OperationKind from the #[rpc] signature
- Add streaming RPCs and Responder return-path migration
- Enhance DaemonBuilder with service registration and refactor component handling
- Add #[component] macro for system-constructed components and enhance field injection
- Implement Component metadata trait and refactor component registration
- Add support for RPC groups and enhance component resolution logic
- Add support for manual component registration and validation in the daemon
- Add overseer-macros crate and implement service and rpc procedural macros
