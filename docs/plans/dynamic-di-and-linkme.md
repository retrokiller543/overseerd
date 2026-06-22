# Plan: Richer DI, Dynamic providers, compile-time validation, and the linkme migration

**Status**: Proposal (not yet approved for implementation)
**Date**: 2026-06-22
**Covers**: TODO #5 (`Arc<dyn T>` / `Vec<Arc<dyn T>>` / `HashMap<String, Arc<dyn T>>`, `provide =`,
qualifiers, primary) plus three additions requested alongside it:
`Optional<T>`, `Dynamic<T>`, descriptors-as-type-system / compile-time validation, and an
`inventory` → `linkme` migration with one static slice per descriptor kind.

---

## 1. Why these belong in one plan

The current DI system supports exactly **one** kind of dependency:

> a *required* *singleton*, resolved by *exact concrete type*, validated at *daemon build (startup)*.

Field injection (`crates/macros/src/inject.rs`) hard-codes this: an `Arc<T>` field is a required
dependency, anything else is `Default::default()`. The runtime container
(`crates/core/src/container.rs`) keys instances `HashMap<TypeId, BoxedComponent>` — strictly one
instance per type — and `validate_dependencies` only checks presence by `TypeId`.

A dependency edge has **three orthogonal axes**, and the node being provided has a **fourth**
(its lifetime). These must stay separate fields, not a single `DependencyKind` enum — folding them
together is a combinatorial explosion (cardinality × provenance × optionality × scope) and forbids
the natural combinations (a `Dynamic`, `Request`-scoped collection is just three markings).

- **Cardinality** (edge): `One | Collection | Keyed | Primary`.
- **Provenance** (edge): static (compile-validated) vs `Dynamic` (runtime-provided).
- **Optionality** (edge): present-or-`None`.
- **Scope / lifetime** (node): `Singleton | Connection | Request | Transient` — see §2b.

Cardinality (cols) against provenance (rows):

| | static core (compile-validated) | dynamic tail (`Dynamic<…>`) |
|---|---|---|
| **exactly one** | `Arc<T>` *(today)* | `Dynamic<T>` |
| **optional** | `Option<Arc<T>>` | `Option<Dynamic<T>>` |
| **many (unkeyed)** | `Vec<Arc<dyn Trait>>` | runtime-registered providers |
| **many (keyed)** | `HashMap<String, Arc<dyn Trait>>` | runtime-registered providers |
| **one-of-many (trait)** | `Arc<dyn Trait>` (`#[primary]` / sole) | — |

Because the macro reads cardinality/optionality/provenance off the wrapper type and the leaf handle
off `Injectable` (§2c), we replace the "`Arc<T>` else `Default`" heuristic with an explicit, total
mapping. This is also what makes compile-time validation honest: the only thing that forces runtime
resolution is an explicit `Dynamic<T>` (or a `with_component` registration), so everything else is
provably complete at compile time.

Both Many unkeyed and keyed support zero providers (empty `Vec`/`HashMap`).

---

## 2. The dependency model

### 2a. Everything is `T: Injectable`; local state is `#[default]`

The governing rule, replacing the old "`Arc<T>` is a dep, anything else is `Default`" heuristic:

> **Every field of a `#[component]`/`#[service]` is a dependency of some `T: Injectable`, resolved
> from the container — unless it is annotated `#[default]`, which makes it local owned state built
> with `Default::default()`.**

`Injectable` is the trait every resolvable handle implements. The blanket impl makes all existing
`Arc<T>` fields valid with zero churn; a type that is *internally* `Arc` (a pool, a client) opts in
for itself so it can be injected **by value** without an outer `Arc`:

```rust
/// How a resolved dependency is stored in, and handed out of, the container.
/// `Target` is the TypeId key the instance is stored under; cloning the handle
/// must be cheap (it is performed on every resolution).
pub trait Injectable: Clone + Send + Sync + 'static {
    type Target: 'static;
}

// Blanket: every `Arc<T>` is injectable, stored/keyed by `T`. Covers all of today's deps.
impl<T: Send + Sync + 'static> Injectable for Arc<T> {
    type Target = T;
}

// Opt-in by-value: a pool that is internally `Arc` is injected directly, no wrapper.
impl Injectable for PgPool {
    type Target = PgPool;   // stored and keyed under itself; `.clone()` is cheap
}
```

We blanket-impl only for `Arc<T>` — **not** `impl<T> Injectable for T` — so there is no coherence
clash with the by-value impls, and a field whose type is neither `Arc<_>` nor a manual `Injectable`
nor `#[default]` fails to compile with a guided message (`#[diagnostic::on_unimplemented]` on
`Injectable`: *"wrap it in `Arc`, implement `Injectable`, or mark the field `#[default]`"*).

`#[default]` is the only escape from injection: the field type must be `Default`, need not be
`Injectable`, and is never looked up in the container. This is the renamed, inverted successor to
today's implicit non-`Arc` branch in `inject.rs`.

### 2b. Cardinality / optionality / provenance: orthogonal wrappers around the `Injectable` leaf

The macro reads three independent edge properties off the wrapper syntax, then takes the leaf handle
(itself `Injectable`):

The wrappers are leaf-agnostic: every one of them takes an `Injectable` handle `H`, which is either
`Arc<T>` (blanket) or a by-value internally-`Arc` type (`PgPool`). So each row below has both an
`Arc<…>` and a by-value spelling.

| Field syntax | cardinality | optional | dynamic | leaf handle |
|---|---|---|---|---|
| `#[default] x: T` | — (local state) | — | — | — |
| `repo: Arc<Repo>` / `pool: PgPool` | One | no | no | `Arc<Repo>` / `PgPool` |
| `cache: Option<Arc<Cache>>` / `pool: Option<PgPool>` | One | **yes** | no | `Arc<Cache>` / `PgPool` |
| `cfg: Dynamic<Config>` / `pool: Dynamic<PgPool>` | One | no | **yes** | `Arc<Config>` / `PgPool` |
| `cfg: Option<Dynamic<Config>>` | One | **yes** | **yes** | `Arc<Config>` |
| `plugins: Vec<Arc<dyn Plug>>` | **Collection** | no | no | `Arc<dyn Plug>` |
| `routes: HashMap<String, Arc<dyn Route>>` | **Keyed** | no | no | `Arc<dyn Route>` |
| `repo: Arc<dyn Repo>` | **Primary** (or sole) | no | no | `Arc<dyn Repo>` |

`Option<H>` and `Dynamic<H>` therefore work uniformly over by-value injectables — `Option<PgPool>` is
an optional by-value pool, `Dynamic<PgPool>` a runtime-provided one — exactly as they do over
`Arc<T>`. These are *separate fields* on the descriptor, never one enum — so e.g. `Option<Dynamic<…>>` or a
`Dynamic` collection compose without new variants:

```rust
#[derive(Clone, Copy, Debug)]
pub enum Cardinality { One, Collection, Keyed, Primary }

#[derive(Clone, Copy, Debug)]
pub struct DependencyDescriptor {
    pub name: &'static str,
    pub ty: TypeDescriptor,    // for trait deps, TypeId::of::<dyn Trait + Send + Sync>()
    pub cardinality: Cardinality,
    pub optional: bool,        // Option<…>
    pub dynamic: bool,         // Dynamic<…> — runtime-provided, skip static validation
}
```

`Dynamic<…>` carries `dynamic: true` so compile-time validation **skips** it and the static-graph
boot optimisation (§5c) treats it as an unknown edge — the explicit escape hatch that makes the rest
of the graph provably complete. `TypeId::of::<dyn Trait + Send + Sync>()` is a stable `'static` key,
so trait deps slot into the same `TypeId`-keyed machinery as concrete deps.

### 2c. Scope is a property of the node, not the edge

Lifetime belongs on the *component being provided*, so it replaces today's `ComponentScope`:

```rust
#[derive(Clone, Copy, Debug)]
pub enum Scope {
    Singleton,    // built once at startup; lives in the root container (today's default)
    Connection,   // one instance per accepted connection; dropped on close
    Request,      // one instance per RPC call; dropped when the call completes
    Transient,    // built fresh on each resolution
}
```

**Lifetime rule (validateable, ideally compile-time):** a node may depend only on equal-or-longer-
lived nodes. Ordering `Singleton > Connection > Request > Transient`. A `Singleton` holding a
`Request`-scoped dependency is an error — surfaced as a `Provide`-style bound (§5) where possible,
else at build.

**This unifies DI with the existing extractors.** `Extension<T>` is already connection-scoped state
and `Payload`/`Conn` are request data — connection/request-scoped *components* are the same lifetimes
built by the graph instead of by hand. Concretely it means scoped resolution contexts layered over
the root container: a per-connection sub-context created at accept time (parent = root) and a
per-call sub-context at dispatch time (parent = connection), with `resolve` walking the parent chain.
`RpcCallContext` already carries `Arc<ComponentContainer>` + connection info, so it is the natural
home. **This is the heaviest architectural piece in the plan** and is sequenced last among the DI
work (see §9) — Phase 0 ships `Singleton`/`Transient` only, with `Connection`/`Request` as a follow-on.

---

## 3. Providers: `provide =`, qualifiers, `#[primary]`

A concrete component opts into being discoverable under a trait:

```rust
#[component(provide = dyn Repo)]
struct PgRepo { pool: Arc<PgPool> }

#[component(provide = [dyn Repo, dyn HealthCheck], qualifier = "pg", primary)]
struct PgRepo { /* ... */ }
```

This means the component, in addition to registering under its own concrete `TypeId`, **also**
registers a *provider entry* under each named trait's `TypeId`. A new descriptor:

```rust
pub struct ProviderDescriptor {
    pub trait_ty: TypeDescriptor,     // TypeId::of::<dyn Trait>()
    pub concrete_ty: TypeDescriptor,  // the providing component's TypeId
    pub qualifier: Option<&'static str>,
    pub primary: bool,
    /// Casts the stored Arc<Concrete> to Arc<dyn Trait> and inserts it under trait_ty.
    pub erase: fn(&BoxedComponent) -> BoxedComponent,
}
```

The `erase` fn pointer is the one piece the macro must generate per `(component, trait)` pair,
because only the macro site knows both the concrete type and that it implements `Trait` — it emits a
monomorphised cast `Arc<Concrete> -> Arc<dyn Trait>`. `Arc<dyn Trait + Send + Sync>` is itself a
concrete `'static` type implementing `Any`, so it boxes and `downcast_ref::<Arc<dyn Trait>>()`s
exactly like today's `Arc<T>` — the container mechanism extends without a redesign.

---

## 4. Container changes

`ComponentContainer` / `ComponentConstructionContext` move from "one instance per `TypeId`" to two
layers:

1. **Instances** — `HashMap<TypeId, BoxedComponent>`, as today (concrete singletons).
2. **Providers** — `HashMap<TypeId /*trait*/, Vec<ProviderEntry>>` where
   `ProviderEntry { qualifier: Option<&'static str>, primary: bool, value: BoxedComponent }`.

After a concrete component is constructed, its `ProviderDescriptor`s run their `erase` fn and push a
clone of the `Arc` into the provider multimap. Resolution then becomes:

- `resolve::<T>() -> Option<Arc<T>>` — unchanged (layer 1).
- `resolve_primary::<dyn Trait>() -> Option<Arc<dyn Trait>>` — the `primary` entry, or the sole
  entry; ambiguity is an error.
- `resolve_all::<dyn Trait>() -> Vec<Arc<dyn Trait>>` — every entry.
- `resolve_keyed::<dyn Trait>() -> HashMap<String, Arc<dyn Trait>>` — every entry with a qualifier.

Topological sort already keys on `TypeId`; providers add edges from a consumer to *each* concrete
provider of the trait it depends on. For `Vec`/`HashMap` deps the edge is "all providers of the
trait must be built first" — straightforward extension of `validate_dependencies` and
`topological_sort`.

---

## 5. Descriptors in the type system + compile-time validation

Two complementary mechanisms, both building on what already exists (`Component`/`ServiceComponent`
already put `ID`/`NAME`/`VERSION` on the type).

### 5a. A `Wiring` trait carrying the graph at the type level

`#[component]`/`#[service]` additionally emit:

```rust
impl Wiring for PgRepo {
    type Deps = (Arc<PgPool>,);      // required static deps
    type Provides = (dyn Repo,);     // traits this provides
    const DYNAMIC_DEPS: &'static [&'static str] = &[]; // names of Dynamic<T> deps, for diagnostics
}
```

This is the "descriptors as part of the type system" idea: the dependency edges become associated
types, not just runtime `&'static [DependencyDescriptor]`. It is a *second encoding* of information
also present in `DependencyDescriptor` — accepted cost, flagged in §8.

### 5b. Compile-time discharge of the static graph

A `Provide<T>` bound, checked by the trait solver, with a readable error:

```rust
#[diagnostic::on_unimplemented(
    message = "no provider for `{T}` — register one, or mark the dependency `Dynamic<{T}>`",
)]
pub trait Provide<T: ?Sized> {}
```

The aggregation point that can see the whole static set in one place is the open decision in §7.
Whatever the form, the rule is: **every `Singleton`/`Primary`/`Collection`/`Keyed` dep must be
discharged at compile time; `Dynamic` deps are exempt and fall through to the existing runtime
`validate()`.** This is precisely the "compile-time except for conditional deps" boundary.

What is *not* reachable at compile time (documented limits, not bugs):
- **Cross-crate** provider/RPC uniqueness — blocked by the orphan rule + link-time assembly. Stays a
  startup check.
- **RPC-path** dedup across `#[handlers]` blocks — cross-invocation aggregation. Stays a startup
  check (already loud and deterministic in `validate_services`).

### 5c. Boot optimisation (optional, later)

Once the static graph is known and contains no `Dynamic` edges, the topological order is computable
at macro/`app!` time, so the runtime `topological_sort` can be skipped (or asserted) for the static
core. `Dynamic<T>` is the *only* thing that forces a runtime resolution pass. This is the concrete
payoff of "leverage the type system for faster boots."

---

## 6. `inventory` → `linkme` migration

Today: one enum `Descriptor { Component | Service | Rpcs }` collected via `inventory::collect!`, with
macros emitting `inventory::submit!` and `DescriptorRegistry::collect()` matching on the variant
(`crates/core/src/descriptors/mod.rs`, `registry/mod.rs:33`).

Target: one `#[distributed_slice]` per descriptor kind, declared in `overseer-core` and re-exported
through the facade so macro-generated code references a stable path:

```rust
#[distributed_slice]
pub static COMPONENTS: [ComponentDescriptor];
#[distributed_slice]
pub static SERVICES: [ServiceDescriptor];
#[distributed_slice]
pub static RPC_GROUPS: [RpcGroup];
#[distributed_slice]
pub static PROVIDERS: [ProviderDescriptor];   // new in this plan
```

`collect()` becomes a direct `COMPONENTS.iter().copied().collect()` per slice — no enum match, each
slice homogeneous and independently reasoned about. Benefits aligned with this plan:
- Homogeneous slices are simpler to reason about and to feature-gate (TODO #14: a
  `manual-registration` build just doesn't link a slice).
- linkme is link-time, not constructor-time, so there is no per-startup registration walk — it
  composes with the §5c "graph known before `main`" direction.

**Migration risks to validate in a spike before committing:**
- linkme requires the slice symbol be retained; dead-code stripping / `--gc-sections` and some
  linkers need care. Confirm on the project's Linux + macOS targets.
- `#[distributed_slice]` elements must be `const`/`'static` (descriptors already are — they are
  `Copy` statics today).
- The macro crate must emit `#[overseer::COMPONENTS]`-style element attributes referencing the
  facade path; `paths::overseer_path` already centralises this.
- `inventory` and `linkme` can coexist during migration, so it can land behind the others without a
  flag day.

---

## 7. Open decisions (resolve before/within implementation, not blocking the plan)

1. **Optional surface**: `Option<Arc<T>>` (no new type, idiomatic) vs `Optional<T>` (explicit,
   parallel). *Leaning `Option<Arc<T>>`.*
2. **Compile-time aggregation point** for §5b: an `app!`-style manifest listing the static core
   (explicit, one place, also gives compile-time dedup of the listed set) vs per-type
   const-assertions emitted by each macro (no manifest, but only within-crate and noisier). *Leaning
   `app!` for the first-party core, runtime for the `Dynamic`/plugin tail — the hybrid.*
3. **`Arc<dyn Trait>` ambiguity policy**: with two non-primary providers and no `#[primary]`, is it a
   hard error or last-wins? *Leaning hard error (consistent with `DuplicateComponentType`).*
4. **Cross-crate providers**: do third-party crates register providers? If yes, those edges are
   link-time only and exempt from compile-time discharge (treated like `Dynamic`).
5. **Provider scope**: do providers inherit the component's `Singleton` scope only, or also
   `Transient`? *Singleton-only for v1.*

---

## 8. Risks & costs

- **Second source of truth**: §5a's type-level `Deps`/`Provides` duplicate `DependencyDescriptor` /
  `ProviderDescriptor`. They must be generated from the same macro pass so they cannot drift; a test
  that asserts the type-level and runtime views agree is cheap insurance.
- **linkme portability** (§6) — spike first.
- **`dyn` downcast ergonomics** — `Arc<dyn Trait + Send + Sync>` must consistently carry the same
  auto-trait bounds at registration and resolution or the `TypeId` differs; centralise the bound in
  one alias.
- **Diagnostic quality** — an undischarged `Provide<T>` bound deep in generated code can read worse
  than the current named `MissingDependency` startup error; `#[diagnostic::on_unimplemented]`
  mitigates but should be reviewed on real failures.
- **Scope** — this is a large, multi-crate change. Sequencing (§9) keeps each step shippable.

---

## 9. Suggested sequencing (each phase independently shippable)

- **Phase 0 — markers, backwards-compatible.** Add `DependencyKind`; teach field-injection +
  `#[init]` to read `Option<Arc<T>>` and `Dynamic<T>`; thread `optional`/`kind` through descriptors;
  runtime `validate()`/`topological_sort` honour them. Keep `inventory`. No `dyn` yet.
- **Phase 1 — linkme migration.** Mechanical, isolated: replace the enum + `collect!` with four
  distributed slices; `collect()` reads each. No behaviour change.
- **Phase 2 — dyn providers.** `provide =` / `qualifier` / `#[primary]`; `ProviderDescriptor` +
  `PROVIDERS` slice; container provider multimap; `Vec`/`HashMap`/`Arc<dyn Trait>` injection;
  ambiguity policy. This is the bulk of TODO #5.
- **Phase 3 — compile-time validation.** `Wiring` trait + `Provide<T>` bounds + the §7.2 aggregation
  point; `Dynamic<T>` as the documented carve-out. CI now catches missing static providers.
- **Phase 4 — boot optimisation (optional).** Compile-time topo-order for the `Dynamic`-free static
  core; skip/assert the runtime sort.

Phases 0–2 deliver TODO #5 + the markers with zero compile-time-validation risk; Phase 3 is where the
"errors at build, not in prod" payoff lands; Phase 4 is pure optimisation on top.
