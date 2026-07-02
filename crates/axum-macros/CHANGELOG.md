# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
