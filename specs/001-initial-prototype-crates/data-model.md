# Data Model: Initial Prototype Crates

## Component Descriptor

**Purpose**: Describes a reusable dependency that can be registered with a daemon definition and inspected by developers.

**Fields**:
- `id`: Stable component identifier unique within a daemon definition.
- `name`: Human-readable component name for diagnostics and inspection.
- `description`: Short explanation of what dependency the component provides.
- `provided_contract`: Optional summary of the value or capability the component contributes.

**Relationships**:
- May be referenced by one or more Dependency Relationships.
- Belongs to one Daemon Definition.

**Validation Rules**:
- `id` must be non-empty and unique within the daemon definition.
- `name` must be non-empty.
- `description` must make the component's purpose clear enough for inspection.

## Service Descriptor

**Purpose**: Describes daemon functionality exposed by a service.

**Fields**:
- `id`: Stable service identifier unique within a daemon definition.
- `name`: Human-readable service name.
- `description`: Summary of the daemon capability represented by the service.
- `operations`: Ordered list of RPC Operation Descriptors owned by the service.

**Relationships**:
- Owns zero or more RPC Operation Descriptors.
- Belongs to one Daemon Definition.

**Validation Rules**:
- `id` must be non-empty and unique within the daemon definition.
- `name` must be non-empty.
- A service used in the prototype demonstration must expose at least one operation.

## RPC Operation Descriptor

**Purpose**: Describes a callable service operation and its typed contract summary without requiring a production transport.

**Fields**:
- `id`: Stable operation identifier unique within its service.
- `name`: Human-readable operation name.
- `description`: Summary of the operation's business behavior.
- `input_contract`: Human-readable summary of accepted input data.
- `output_contract`: Human-readable summary of returned output data.
- `dependencies`: References to Dependency Relationships or required component identifiers.

**Relationships**:
- Belongs to one Service Descriptor.
- May depend on zero or more Component Descriptors.
- May appear in inspection output and future client-generation contracts.

**Validation Rules**:
- `id` must be non-empty and unique within the owning service.
- `input_contract` and `output_contract` must be present for prototype operations.
- Referenced component dependencies must exist in the owning Daemon Definition.

## Dependency Relationship

**Purpose**: Describes how an operation or service depends on registered components or framework-provided context.

**Fields**:
- `consumer_id`: Identifier of the service or operation that requires the dependency.
- `provider_id`: Identifier of the component or context provider satisfying the dependency.
- `reason`: Short explanation of why the dependency is required.

**Relationships**:
- Connects RPC Operation Descriptors or Service Descriptors to Component Descriptors.
- Belongs to one Daemon Definition.

**Validation Rules**:
- Both consumer and provider identifiers must resolve within the daemon definition.
- `reason` must be non-empty for inspectability.

## Daemon Definition

**Purpose**: Represents the assembled prototype daemon metadata that a user-owned startup flow can hand to Overseer.

**Fields**:
- `name`: Daemon name used in diagnostics and inspection.
- `components`: Registered Component Descriptors.
- `services`: Registered Service Descriptors.
- `dependencies`: Dependency Relationships derived from services and operations.
- `inspection_summary`: Human-readable view of registered metadata categories.

**Relationships**:
- Owns all descriptors and dependency relationships for one daemon.
- Is produced through explicit registration in the prototype.

**Validation Rules**:
- `name` must be non-empty.
- Component identifiers must be unique.
- Service identifiers must be unique.
- Operation identifiers must be unique within their service.
- Dependency references must resolve before the definition is considered valid.

## Crate Boundary

**Purpose**: Records the single responsibility of each prototype crate and what behavior is intentionally excluded.

**Fields**:
- `crate_name`: Package name.
- `path`: Repository-relative crate path under `crates/`, except for the root facade package.
- `responsibility`: One-sentence responsibility statement.
- `owns`: Behaviors the crate may implement.
- `does_not_own`: Behaviors that must stay outside the crate.

**Planned Boundaries**:
- `overseer` at repository root: facade and re-export entrypoint only; does not own core implementation.
- `overseer-core` at `crates/overseer-core`: metadata descriptors, explicit registration, daemon definitions, validation, and inspection models; does not own examples, macros, production transports, SDK generation, or external process/runtime management.
- `overseer-demo` at `crates/overseer-demo`: prototype demonstration and validation fixture; does not own framework core behavior.

**Validation Rules**:
- All implementation crates must live under `crates/`.
- Each implementation crate must have exactly one primary responsibility.
- A behavior must not be owned by more than one crate.

## Prototype Demonstration

**Purpose**: Proves the first vertical slice from explicit registration through inspection.

**Fields**:
- `daemon_name`: Name of the demonstration daemon.
- `component`: Example Component Descriptor.
- `service`: Example Service Descriptor.
- `operation`: Example RPC Operation Descriptor.
- `dependency`: Example Dependency Relationship.
- `expected_inspection_categories`: Components, services, operations, dependencies, and daemon-level registration.

**Relationships**:
- Uses `overseer-core` descriptors and registration APIs.
- Lives in `overseer-demo`.

**Validation Rules**:
- Must exercise at least one component-backed service operation.
- Must produce inspectable output showing all required metadata categories.
- Must not require network, persistence, credentials, or framework ownership of application startup.
