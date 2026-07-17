# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.16.0](https://github.com/retrokiller543/overseerd/compare/overseerd-transport-v0.15.0...overseerd-transport-v0.16.0) - 2026-07-17

### Fixed

- *(runtime)* harden lifecycle, reloads, and filesystem safety ([#85](https://github.com/retrokiller543/overseerd/pull/85))

## [0.12.0](https://github.com/retrokiller543/overseerd/compare/overseerd-transport-v0.11.2...overseerd-transport-v0.12.0) - 2026-07-08

### Other

- Added docs to all crates

## [0.10.0](https://github.com/retrokiller543/overseerd/compare/overseerd-transport-v0.9.1...overseerd-transport-v0.10.0) - 2026-07-03

### Added

- Made the framework compile to wasm and be able to generate wasm rest clients for axum.

## [0.7.0](https://github.com/retrokiller543/overseerd/compare/overseerd-transport-v0.6.0...overseerd-transport-v0.7.0) - 2026-06-30

### Other

- Feature/protocol agnostic ([#20](https://github.com/retrokiller543/overseerd/pull/20))

## [0.4.0](https://github.com/retrokiller543/overseerd/compare/overseerd-transport-v0.3.0...overseerd-transport-v0.4.0) - 2026-06-25

### Other

- directory-namespace ergonomics + tagged-enum defaults ([#10](https://github.com/retrokiller543/overseerd/pull/10))

## [0.1.1](https://github.com/retrokiller543/overseerd/compare/overseerd-transport-v0.1.0...overseerd-transport-v0.1.1) - 2026-06-24

### Other

- release v0.1.0

## [0.1.0](https://github.com/retrokiller543/overseerd/releases/tag/overseerd-transport-v0.1.0) - 2026-06-24

### Fixed

- fixed formating and fixed macro paths

### Other

- add crate metadata and internal dep versions for publishing
- renamed the project to overseerd
- Add configuration system and application directories
- Add support for streaming with custom codecs and enhance client call handling
- Add client SDK generation support and enhance error handling
- Implement structured status codes for RPC error responses
- Add streaming RPCs and Responder return-path migration
- Enhance DaemonBuilder with service registration and refactor component handling
- Cleaned up the protocol layer
- Added rough transport and protocol impl
