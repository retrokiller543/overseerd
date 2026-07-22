# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.20.0](https://github.com/retrokiller543/overseerd/compare/overseerd-macros-core-v0.19.1...overseerd-macros-core-v0.20.0) - 2026-07-22

### Added

- *(rpc,axum)* dual linkme/inventory backend for rpc groups & routes; stable hook order
- *(di)* dual linkme/inventory registration backend for factories & hooks

### Other

- *(hooks)* non_exhaustive HookDescriptor + constructor; doc factory_slice merge

## [0.19.1](https://github.com/retrokiller543/overseerd/compare/overseerd-macros-core-v0.19.0...overseerd-macros-core-v0.19.1) - 2026-07-21

### Fixed

- *(provider)* enhance descriptor handling for provider traits

## [0.19.0](https://github.com/retrokiller543/overseerd/compare/overseerd-macros-core-v0.18.0...overseerd-macros-core-v0.19.0) - 2026-07-21

### Added

- *(di)* add provider ordering and deferred primitives

### Fixed

- *(di)* hydrate deferred dependencies after build

## [0.14.2](https://github.com/retrokiller543/overseerd/compare/overseerd-macros-core-v0.14.1...overseerd-macros-core-v0.14.2) - 2026-07-17

### Other

- *(di)* harden hot paths and add controlled benchmarks

## [0.12.0](https://github.com/retrokiller543/overseerd/compare/overseerd-macros-core-v0.11.2...overseerd-macros-core-v0.12.0) - 2026-07-08

### Other

- Added docs to all crates

## [0.11.0](https://github.com/retrokiller543/overseerd/compare/overseerd-macros-core-v0.10.0...overseerd-macros-core-v0.11.0) - 2026-07-07

### Added

- *(axum)* drop custom guards from client codegen; add query/raw/multipart bodies + per-call & transport headers

## [0.10.0](https://github.com/retrokiller543/overseerd/compare/overseerd-macros-core-v0.9.1...overseerd-macros-core-v0.10.0) - 2026-07-03

### Added

- Made the framework compile to wasm and be able to generate wasm rest clients for axum.

## [0.7.0](https://github.com/retrokiller543/overseerd/compare/overseerd-macros-core-v0.6.0...overseerd-macros-core-v0.7.0) - 2026-06-30

### Other

- Feature/protocol agnostic ([#20](https://github.com/retrokiller543/overseerd/pull/20))
