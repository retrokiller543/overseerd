# Plugin Composition Identity And Resolution Foundation

## Outcome

Deliver the first reviewable implementation slice from the accepted architecture in PR `#161` and issue `#176`:

- stable protocol, plugin, plugin-slot, and contribution identities;
- installation and contribution provenance;
- public structural declarations for dependencies, conflicts, ordering, replacement, and suppression;
- deterministic early and late plugin resolution;
- typed, aggregate composition diagnostics;
- an immutable plugin-resolution plan that later slices can lower into the application registries.

This slice is architectural groundwork for `#146`. It does **not** change current runtime behavior, replace `ProtocolPlugin`, migrate RPC/Axum/Jobs, alter scope handling, retain plugin values in `AppBuilder`, lower contributions, or add CLI providers.

## Delivery Setup

Perform these steps before source edits:

1. Create a focused GitHub implementation issue under `#146` titled `Plugin architecture: composition identity and deterministic resolution`. Copy this plan's outcome, contracts, validation gates, and exclusions into the issue.
2. Create a separate issue for the existing scope-topology correctness defect described in `#176`. It must cover namespaced scope identity, declared parents, open-time checks, Axum HTTP/WebSocket paths, and reachability validation, but is not implemented by this PR.
3. Fetch `origin/feat/141-149-app-cli-tooling` and create the issue branch from that remote head, not from the currently checked-out `feat/app-cli/145/typed-commands` branch. Use `feat/app-cli/<issue-number>/composition-identity`.
4. Stage only `.kilo/plans/1784923645046-plugin-composition-foundation.md`, preserving the unrelated untracked plan/doc files already in the worktree.
5. Commit the plan by itself with `mise exec -- git commit -m "docs(plugin): plan deterministic composition foundation"`, push the branch, and open a draft PR targeting `feat/141-149-app-cli-tooling`.
6. Add implementation commits to that issue branch. This additive slice does not require a breaking-change commit marker unless implementation changes an existing public contract unexpectedly.
7. Never merge the PR. Resolve addressed review conversations and leave approval/merge to the project owner.

## Settled Contracts

### Stable IDs

Add category-safe newtypes in `overseerd-app`:

- `ProtocolId`: selected protocol definition identity.
- `PluginId`: exact plugin implementation identity.
- `PluginSlotId`: a replaceable capability/default slot, distinct from its implementation.
- `ContributionId`: contributor-local contribution identity; full identity is `(Contributor, ContributionId)`.

All IDs use a validated lowercase ASCII namespaced path such as `overseerd/rpc` or `acme/radix-router`:

- at least two non-empty `/`-separated segments;
- each segment starts with `[a-z0-9]` and continues with `[a-z0-9._-]`;
- comparison and ordering use canonical bytes;
- `overseerd/...` is reserved for framework-owned definitions;
- IDs are explicit and never inferred from `TypeId`, `type_name`, crate names, dependency aliases, display names, or discovery order.

Provide one validated construction path shared by runtime and const/static declaration use. Do not expose an unchecked public constructor. Each newtype provides `as_str`, `Display`, and `Copy + Clone + Debug + Eq + Ord + Hash`.

### Identity And Cardinality

- One `PluginId` may be installed at most once in an effective plan. Repeated installation is a typed duplicate diagnostic.
- A plugin installation may occupy at most one `PluginSlotId` in this slice.
- Dependencies, conflicts, `before`, and `after` relations use an explicit target enum: exact `PluginId` or effective `PluginSlotId`.
- Exact-plugin relations do not follow replacement. Slot relations resolve to whichever effective implementation occupies that slot.
- A contribution ID is unique only within its contributor. Repeated semantically distinct contributions require distinct stable IDs or one grouped contribution later; no generated ordinal is treated as contribution identity.

### Contributors And Provenance

Model contributors explicitly rather than fabricating synthetic plugins:

- framework;
- application declaration;
- selected protocol (`ProtocolId`);
- plugin (`PluginId`).

Installation provenance records:

- origin: framework/protocol default, static application declaration, or late application configuration;
- declaration ordinal scoped to that origin's declaration list;
- early or late phase.

The ordinal is diagnostic/provenance data only. It never decides plugin order. Backend fixtures retain their original ordinals when their collection order is permuted.

`ContributionProvenance` publicly combines `Contributor` and `ContributionId`. Actual component/provider/config/protocol payloads and uniqueness checks wait for contribution lowering in Slice 4.

### Slot Policy And Directives

Use a finite slot policy enum:

- fixed: cannot be replaced or disabled;
- replaceable: may be replaced but not disabled;
- optional: may be replaced or explicitly disabled.

Structural directives are explicit installs, replacements, and suppressions:

- a normal second provider for an occupied slot is a duplicate-slot error, never last-wins;
- replacement names the target slot and a differently identified implementation;
- replacement is resolved before dependency/conflict/order validation;
- multiple replacements of one slot are errors;
- suppression is valid only for optional slots;
- dependencies on a suppressed slot are missing dependencies;
- conflicts are evaluated against the effective post-replacement/post-suppression set.

### Early And Late Composition

Support dynamic configuration without allowing parser/tooling drift:

1. **Early declarations** come from framework/protocol defaults and static application declarations. Resolve them before argument parsing. This plan is the source for future CLI and pre-parse tooling projections.
2. **Late declarations** may be installed during application configuration. They extend the early plan before the final preparation freeze.
3. Late plugins may depend on, conflict with, or order after early plugins and slots.
4. Late plugins may declare and resolve new late slots, including replacement/suppression among late declarations.
5. Late declarations may not replace or suppress early slots, duplicate early plugin IDs, satisfy an early missing dependency, contribute parser shape, or impose `before` ordering that would move an early plugin.
6. The final order is the unchanged early topological order followed by the late topological order. This phase boundary is semantic and visible in provenance.

Slice 1 represents and validates these phase rules. Slice 4 wires late installation into configuration, and Slice 8 rejects actual CLI facets from late contributors.

### Relation Semantics

Use the following truth table:

| Relation | Missing target | Effective target | Ordering |
| --- | --- | --- | --- |
| hard dependency | typed error | target before dependent | yes |
| `before` / `after` | ignored | declared direction | yes |
| conflict | ignored | typed symmetric conflict | no |
| replacement | typed error | resolve slot policy first | selected implementation inherits slot |

Additional invariants:

- hard dependencies and ordering-only edges remain distinct typed relations;
- self-dependencies and effective self-ordering are typed errors;
- one-sided conflict declaration is sufficient and conflict pairs are canonicalized;
- duplicate graph edges do not increment indegree twice, but retain all relation kinds for diagnostics;
- unconstrained ready plugins sort lexically by `PluginId`; declaration and discovery order have no semantic effect.

## Public API Boundary

Create a focused module rather than extending the legacy traits:

```text
crates/app/src/composition/
├── mod.rs          # public IDs, provenance, targets, declarations, directives, plans
├── diagnostic.rs   # typed diagnostics and non-empty deterministic report
├── resolver.rs     # normalization, selection, graph resolution, cycle extraction
└── tests.rs        # sibling test module required by repository convention
```

Expose from `overseerd-app` and the non-Wasm facade root, but not the prelude:

- stable ID types and invalid-ID error;
- contributor, phase, origin, and provenance types;
- relation target/kind and slot-policy types;
- immutable plugin declaration/directive constructors;
- early and final plugin-resolution plans with read-only accessors;
- resolver entry points;
- typed diagnostics and diagnostic report.

Mark externally extensible public records/enums `#[non_exhaustive]` where appropriate, keep fields private, and provide constructors/accessors so later contribution fields do not require struct-literal breaks.

Name the Slice 1 result `EarlyPluginPlan` / `PluginResolutionPlan`, not `CompositionPlan`. Reserve `CompositionPlan` for Slice 4's real freeze artifact containing attributed contribution metadata and private executable payload ownership.

Do not re-export these contracts from protocol-specific RPC/Axum crates yet. They are available through direct `overseerd-app` use and the main `overseerd` facade.

## Resolution And Diagnostics

Implement the resolver with standard-library ordered collections; add no graph or random-test dependency.

1. Validate IDs and canonicalize relation lists.
2. Group declarations by `PluginId` and `PluginSlotId`; emit deterministic duplicate diagnostics naming both provenances.
3. Apply replacement and suppression directives in canonical slot/ID order.
4. Validate hard dependencies, conflicts, phase restrictions, and self-relations against the effective set. Ignore absent ordering-only/conflict targets by contract.
5. Lower dependency, `before`, and `after` relations into directed edges (`target -> dependent`, `source -> target`, `target -> source`).
6. Topologically resolve each phase with Kahn's algorithm and a lexical `PluginId` ready set.
7. If nodes remain, run deterministic SCC/DFS analysis. Emit one diagnostic per cyclic SCC with a canonical representative cycle, typed edge kinds, and no unrelated downstream blocked nodes.
8. Return immutable boxed/slice-backed plan data and indexes hidden behind read-only accessors.

Return a non-empty aggregate `CompositionDiagnostics` report rather than the first encountered failure. Collect only independent failures in fixed phases to avoid cascades:

1. invalid IDs, duplicate plugin IDs/slots, and invalid directives;
2. replacement/suppression policy errors;
3. missing dependencies, conflicts, self-relations, and early/late phase violations;
4. graph cycles only when prior structural errors do not make the graph ambiguous.

Sort diagnostics by stable category, target IDs, source IDs, and provenance. Derive/implement structural equality so permutation tests compare typed reports and rendered text.

Required diagnostic categories include:

- invalid ID;
- duplicate plugin and duplicate slot provider;
- missing dependency;
- self dependency/order;
- installed conflict naming both contributors;
- replacement target missing, forbidden, or multiply replaced;
- suppression target missing or not optional;
- early/late phase violation, including late reordering of early nodes;
- dependency/ordering cycle with typed steps.

Do not add a variant to the existing app `Error` yet; no current app operation consumes this resolver until Slice 4.

## Implementation Steps

1. Add `composition` IDs, grammar validation, display/order implementations, and focused ID tests.
2. Add contributor, installation phase/origin/provenance, contribution provenance, relation targets, slot policies, declarations, and directives with private fields and public constructors.
3. Add typed diagnostics and deterministic aggregate report rendering.
4. Implement early normalization, duplicate handling, slot replacement, and optional suppression.
5. Implement relation validation and deterministic early topological resolution.
6. Implement monotonic late-plan extension and phase-boundary diagnostics.
7. Implement canonical cyclic-SCC reporting with preserved relation kinds.
8. Add read-only early/final plan queries for selected protocol, effective plugins, slot occupant, phase, provenance, replacement/suppression decisions, and resolved order.
9. Re-export the public vocabulary from `crates/app/src/lib.rs` and `src/lib.rs` under the existing non-Wasm app gate.
10. Keep current `Plugin`, `ProtocolPlugin`, `AppBuilder::plugin`, `AppRegistry`, `PreparedApp`, protocol accumulators, generated macro bounds, and runtime sequencing unchanged.

## Validation

Add sibling-module tests covering:

- valid/invalid ID grammar, category safety, display, hashing, and lexical ordering;
- exact-plugin versus slot-target behavior through replacement;
- duplicate plugin IDs and slot providers with both origins;
- fixed, replaceable, and optional slot replacement/suppression rules;
- missing/self dependencies and symmetric conflicts;
- `before`/`after`, ignored absent ordering targets, and edge deduplication;
- dependency chains, diamonds, mixed dependency/order graphs, and independent lexical ordering;
- exact cycle membership and canonical paths for dependency, ordering, and mixed cycles;
- early plan stability after late extension;
- allowed late dependency/order-after relations;
- rejected late early-slot replacement/suppression, duplicate IDs, and retroactive ordering;
- contributor-local contribution IDs and provenance equality;
- facade-safe public construction using only exported APIs.

Exhaustively permute small valid and invalid declaration sets while preserving embedded provenance ordinals. Every permutation must produce the same typed plan/report and display text. Add two backend-shaped fixtures with identical semantic records in linkme-like and inventory-like enumeration order; actual registration-backend integration remains Slice 4 because no plugin catalog backend exists yet.

Run the repository gates:

```text
cargo fmt --all -- --check
cargo nextest run -p overseerd-app
cargo nextest run --workspace --all-features
cargo clippy --workspace --all-targets --all-features
cargo check --workspace --no-default-features
cargo check -p overseerd-app --no-default-features
```

No dependency is added, so prohibited dependency paths should remain unchanged. If the manifest changes unexpectedly, also verify `cargo tree --workspace --all-features -i openssl` and `-i rsa` remain empty.

## Completion Gate

- Public third-party-safe identities and structural composition contracts exist without private/facade-only paths.
- Early resolution is invariant under declaration and backend enumeration permutations.
- Late composition extends but cannot invalidate or reorder the early parser-visible plan.
- Replacements preserve distinct implementation provenance while slot-targeted dependencies follow the selected implementation.
- Diagnostics are typed, aggregate, deterministic, and name all relevant contributors.
- No component factory, protocol build, hook, config resolution, transport, or runtime behavior is invoked or changed.
- CI passes and automated review reports no remaining issues; the project owner performs the merge.

## Deferred Slices

- Scope identity/topology and Axum path correctness: separate prerequisite issue/PR.
- First-class `ProtocolDefinition -> PreparedProtocol -> ProtocolRuntime`: Slice 3.
- Retained plugin instances, protocol-default expansion, attributed contribution collection, final `CompositionPlan`, registry lowering, and freeze integration: Slice 4.
- RPC, Axum, and Jobs migrations: Slices 5-7.
- Clap-native pre-parse provider aggregation and late-CLI rejection: Slice 8.
- Serializable tooling projections and deterministic plan hash: Slice 9.
- Removal of `ProtocolPlugin`, `RpcPlugin`, and `AxumPlugin` and migration documentation: Slice 10.
