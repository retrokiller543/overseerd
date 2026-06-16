# Core Registration Contract

## Purpose

Defines the behavior expected from the prototype core crate. The core contract is descriptor-first and explicit-registration-first.

## Consumers

- Root `overseer` facade package.
- `overseer-demo` prototype demonstration crate.
- Future runtime, macro, transport, and SDK-generation crates.

## Required Capabilities

1. Register a component descriptor with a daemon definition builder.
2. Register a service descriptor with a daemon definition builder.
3. Attach at least one RPC operation descriptor to a service descriptor.
4. Record dependency relationships between operations or services and components.
5. Validate that descriptor identifiers are non-empty and unique within the appropriate scope.
6. Validate that dependency references resolve to registered descriptors.
7. Produce an immutable daemon definition after registration succeeds.
8. Expose a read-only inspection view of components, services, operations, dependencies, and daemon-level metadata.

## Error Behavior

- Duplicate identifiers must produce a clear error naming the conflicting identifier and descriptor category.
- Missing dependency providers must produce a clear error naming the unresolved provider identifier.
- Empty required names or identifiers must produce a clear error naming the invalid field.
- Errors must not mention secrets, credentials, or environmental data.

## Out of Scope

- Automatic discovery.
- Procedural macro expansion.
- Production transport routing.
- Generated client SDKs.
- Process supervision or external runtime ownership.
- Persistence or migrations.
