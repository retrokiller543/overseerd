# Upwell Renderer Component Contract

User renderers are WebAssembly **components** implementing the versioned world in
`renderer.wit`. Generate guest bindings from that file with the component tooling
for the guest language. Do not target a core Wasm module or manually depend on the
canonical ABI unless implementing a low-level toolchain adapter.

## Required Component Export

The component must export this WIT function from world
`upwell:renderer/renderer@0.1.0`:

```wit
export render: func(request: render-request) -> result<render-response, string>;
```

The component must have **no imports**. Cargo Upwell supplies no WASI, filesystem,
network, environment, clock, randomness, process, or application host functions.
Any top-level import is rejected before instantiation.

## Request

- `abi-version`: exact renderer host ABI version, currently `0.1.0`.
- `command`: `check`, `doctor`, `inspect`, `export`, `graph`, or `explain`.
- `format`: selected catalog format ID.
- `media-type`: configured output media type.
- `tooling-schema`: semantic version of the canonical input contract.
- `resources`: stable IDs selected by the command and its filters/query.
- `color`: whether ANSI color is permitted for this invocation.
- `payload`: bounded canonical JSON for the selected command projection.

Payloads are `CommandReport`, `ToolingDocument`, `ProbeEnvelope`, `GraphView`, or
`ResourceExplanation` JSON according to `command`. Selection and validation happen
before component execution; renderers cannot alter command outcomes or canonical
documents.

## Response

- `format` must exactly equal the request format.
- `media-type` must exactly equal the request media type.
- `resources` may contain only IDs present in the request.
- `body` contains the bounded rendered output bytes.

When the catalog entry has `utf8 = true`, `body` must be valid UTF-8. A failed
result, trap, deadline, fuel exhaustion, invalid claim, or limit violation produces
an actionable diagnostic and invokes the command's native fallback renderer.

## Low-Level Core Symbols

Generated component bindings normally create these automatically. A component
adapter that implements the canonical ABI directly needs an embedded core module
with:

- exported linear memory named `memory`;
- exported allocator named `cabi_realloc`;
- exported lowered function named `render`;
- exported post-return function named `cabi_post_render` when the lifted result
  owns guest allocations.

These are **adapter implementation details**, not the stable Upwell plugin API.
Their flattened signatures and memory layout are determined by the Component Model
canonical ABI and may change when the WIT types change. The readable fixture at
`../tests/fixtures/renderer/valid.component.wat` demonstrates the current ABI for
test purposes; user renderers should use generated bindings.

## Catalog Registration

Register the resulting `.component.wasm` file as a `renderer` entry in the shared
Upwell `catalog.toml`. See `../catalog.example.toml` for every supported field.
Relative component paths resolve from the catalog directory. Shell completion reads
only catalog metadata and never loads or executes a component.
`UPWELL_CATALOG_PATH` may point every command and completion process at an explicit
catalog file.
