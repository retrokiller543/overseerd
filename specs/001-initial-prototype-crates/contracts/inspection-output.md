# Inspection Output Contract

## Purpose

Defines the information the first prototype must make visible so generated or assembled behavior remains inspectable.

## Required Metadata Categories

The prototype inspection path must show at least:

1. Daemon-level registration metadata.
2. Registered components.
3. Registered services.
4. Registered RPC-style operations.
5. Dependency relationships between operations/services and components.

## Required Properties

For a successful prototype demonstration, inspection output must let a developer answer:

- What daemon definition was assembled?
- Which components are registered?
- Which services are registered?
- Which operations does each service expose?
- What input and output contract summaries are associated with each operation?
- Which components does an operation or service depend on?
- Which crate owns the demonstration and which crate owns the core descriptors?

## Verification Expectations

- Automated tests should assert that the required categories are present in the inspection view.
- The runnable demonstration should print or otherwise expose the categories in a human-readable form.
- The inspection view must not require network access, persistence, credentials, or framework ownership of `main`.

## Out of Scope

- Stable machine-readable schema for external clients.
- Generated API documentation.
- Multi-language SDK contracts.
- Runtime health or metrics output.
