# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0](https://github.com/retrokiller543/overseerd/compare/overseerd-config-v0.4.0...overseerd-config-v0.5.0) - 2026-06-25

### Other

- manager owns the config registry and seeds all defaults (fixes cross-path default references) ([#12](https://github.com/retrokiller543/overseerd/pull/12))

## [0.4.0](https://github.com/retrokiller543/overseerd/compare/overseerd-config-v0.3.0...overseerd-config-v0.4.0) - 2026-06-25

### Other

- directory-namespace ergonomics + tagged-enum defaults ([#10](https://github.com/retrokiller543/overseerd/pull/10))

## [0.3.0](https://github.com/retrokiller543/overseerd/compare/overseerd-config-v0.2.0...overseerd-config-v0.3.0) - 2026-06-25

### Added

- *(config)* select a default enum variant with #[default] ([#8](https://github.com/retrokiller543/overseerd/pull/8))

## [0.2.0](https://github.com/retrokiller543/overseerd/compare/overseerd-config-v0.1.1...overseerd-config-v0.2.0) - 2026-06-25

### Other

- directory namespace, templated field defaults, enum support, app errors & unified logging ([#6](https://github.com/retrokiller543/overseerd/pull/6))

## [0.1.1](https://github.com/retrokiller543/overseerd/compare/overseerd-config-v0.1.0...overseerd-config-v0.1.1) - 2026-06-24

### Other

- release v0.1.0

## [0.1.0](https://github.com/retrokiller543/overseerd/releases/tag/overseerd-config-v0.1.0) - 2026-06-24

### Other

- add crate metadata and internal dep versions for publishing
- renamed the project to overseerd
- Address PR #1 review: correct parse-error variant and cap resolution depth
- Add configuration system and application directories
