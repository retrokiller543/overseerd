# Protocol-Owned Scope Topology

## Outcome

Deliver issue `#178`, the second reviewable plugin-architecture slice tracked by PR `#161`:

- stable namespaced scope identity separate from display labels, Rust types, and lifetime metadata;
- a protocol-owned rooted topology replacing the flat scope chain;
- path-aware dependency and construction validation;
- typed runtime rejection of undeclared scopes, invalid parents, and foreign parents;
- destination-checked dynamic seeds;
- accurate RPC and Axum scope paths.

This slice preserves the current `ProtocolPlugin` and application lifecycle. First-class protocol
states, retained plugin instances, contribution lowering, plugin CLI providers, and tooling
serialization remain later slices.

## Settled Model

### Identity

Add a category-safe `ScopeId` using the established lowercase ASCII namespaced path grammar. Scope
IDs are explicit stable identity. Display names remain diagnostic labels and may be duplicated.
Root and transient use reserved framework IDs and cannot be protocol topology nodes.

Every static scope supplies an ID. All scope maps, comparisons, construction orders, and errors use
the ID rather than `Scope::name()` or rank. Rank remains lifetime metadata during this migration but
does not establish sibling reachability.

### Topology

Replace `ProtocolPlugin::SCOPES` with an immutable protocol-owned rooted topology. Each declared
boundary has exactly one parent, either root or another declared boundary. Validate duplicate IDs,
reserved IDs, missing parents, cycles, and invalid parent lifetime ordering before DI construction.

Represent the protocol paths as:

```text
RPC:  root -> rpc-connection -> rpc-request
Axum: root -> http-request
      root -> websocket-connection -> websocket-message
```

Axum HTTP requests and WebSocket messages use distinct scope identities and marker types. Do not
retain an alias that allows one ambiguous request scope to be used for both paths.

### Reachability And Planning

A component at boundary `C` may depend on a non-transient component at `D` only when `D == C` or
`D` is an ancestor of `C`. Sibling scopes are unreachable regardless of rank. Validate the actual
provider selected for concrete, qualified, collection, deferred, and fresh dependency forms.

Build each boundary's construction order from root, its ancestors, and its local descriptors only.
Factory-less dynamic seed descriptors count as available only at their declared destination and its
descendants. Key all plans by `ScopeId`.

### Runtime Opening

`AppRuntime::open_scope` accepts a declared scope boundary, verifies the requested child, checks the
actual parent against the declared edge, and rejects parents from another runtime. A successful
boundary open always retains its logical scope identity, even when it has no components or seeds;
the old empty-child elision cannot bypass topology checks.

Return typed errors carrying the child, expected parent, actual parent, and relevant stable IDs for:

- undeclared scope opens;
- invalid parent-child opens;
- foreign-runtime parents.

Keep the lower-level DI container construction API protocol-neutral. The topology enforcement point
is the prepared `AppRuntime` used by protocols.

### Dynamic Seeds

Declare protocol-owned dynamic seed destinations separately from factory-backed component
construction. Validate before construction that every seed type is registered for the opened
destination and appears at most once. Report typed invalid-destination and duplicate-seed errors.

Migrate existing seeds:

- RPC `PeerInfo` -> RPC connection;
- Axum `RequestMeta` -> HTTP request;
- WebSocket upgrade metadata -> WebSocket connection;
- JSON WebSocket message context, if any -> WebSocket message;
- STOMP headers, session, and principal -> WebSocket message.

Do not move STOMP principal into connection scope in this slice because it is produced after the
protocol-level `CONNECT` exchange.

## Public Boundary

Expose third-party-safe contracts from their owning crates and the main facade:

- `ScopeId` and invalid-ID error from core;
- immutable scope parent, boundary, topology, and topology-validation contracts from app;
- typed preparation and runtime errors with stable IDs;
- public constructors/accessors sufficient for a third-party `ProtocolPlugin` to declare a topology
  and use `AppRuntime::open_scope` without private paths.

Keep fields private and use constructors/accessors. Do not add topology contracts to protocol-
specific preludes unless already required by their existing public surface.

## Organization

- Keep stable scope vocabulary in `crates/core/src/scope.rs` with sibling tests.
- Extract topology validation and scope construction planning from the large `crates/app/src/app.rs`
  into focused modules.
- Extract DI reachability validation from the large registry implementation rather than extending
  its inline scope logic.
- Keep physical scope storage and resolution in the DI container.
- Keep RPC and Axum topology declarations in their scope modules and opening call sites thin.
- Preserve sibling test modules; do not add inline tests.

## Implementation Sequence

1. Add `ScopeId`, reserved root/transient identities, display-name separation, and public tests.
2. Add immutable rooted topology declarations, validation, ancestry queries, and typed diagnostics.
3. Replace name/rank identity use in descriptors, registry validation, planning, containers, and
   runtime lookup with stable IDs and topology reachability.
4. Add runtime parent and runtime-ownership checks and retain empty logical boundaries.
5. Add destination declarations and validation for dynamic seeds.
6. Migrate RPC to `root -> connection -> request` and preserve existing behavior.
7. Split Axum HTTP request and WebSocket message boundaries and migrate HTTP, JSON WebSocket, and
   STOMP opening/seeding paths.
8. Add public/facade-only third-party topology coverage and targeted feature-gated tests.

## Validation

Required behavior:

- duplicate display labels with different IDs do not alias;
- duplicate IDs and malformed topologies return typed preparation errors;
- undeclared, wrong-parent, and foreign-runtime opens return typed runtime errors;
- empty boundaries retain identity;
- HTTP request components cannot depend on WebSocket connection components;
- WebSocket message components can depend on WebSocket connection components;
- RPC connection/request scopes retain current lifetime behavior;
- wrong-destination, unregistered, and duplicate dynamic seeds fail before construction;
- public direct-crate and facade paths support a third-party topology;
- `ws`-disabled Axum does not declare dormant WebSocket boundaries.

Run:

```text
cargo fmt --all -- --check
cargo nextest run --workspace --all-features
cargo clippy --workspace --all-targets --all-features
cargo check --workspace --no-default-features
cargo check -p upwell-app --no-default-features
cargo check -p upwell-axum --no-default-features
cargo check -p upwell-axum --no-default-features --features ws
cargo check -p upwell-axum-json-ws --all-features
cargo check -p upwell-axum-stomp --all-features
```

Retain all existing CI Wasm checks because server scope contracts remain feature-gated out of Wasm
client builds.

## Delivery

1. Commit this plan alone on `feat/app-cli/178/scope-topology` using the required mise Git profile.
2. Push and open a draft PR against `feat/141-149-app-cli-tooling` before source changes.
3. Use `BREAKING CHANGE: ` for commits that intentionally change existing public scope contracts.
4. Resolve every automated review conversation after addressing it.
5. Never merge the PR; approval and merge remain with the project owner.

## Exclusions

- `ProtocolDefinition -> PreparedProtocol -> ProtocolRuntime` migration;
- removal or compatibility adaptation of `ProtocolPlugin`, `RpcPlugin`, or `AxumPlugin`;
- retained plugin catalogs, contribution collection, immutable composition freeze, or registry
  lowering;
- plugin-defined runtime boundaries;
- deferred post-handshake WebSocket connection-scope construction;
- optional RPC/Axum capability extraction;
- CLI contribution aggregation or tooling serialization.
