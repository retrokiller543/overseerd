---
name: upwell-documentation-audit
description: Audit Upwell's release documentation for missing, incomplete, stale, or low-quality coverage, maintain the local coverage manifest, and report every verified finding as a deduplicated GitHub issue in upwell-rs/upwell-docs.
---

# Upwell Documentation Audit

Audit the published Upwell documentation at `https://upwell-docs-production.up.railway.app/docs/<release>` against the matching public framework release. Use this skill when asked to identify undocumented Upwell features, assess documentation completeness or quality, or report documentation gaps.

## Non-negotiable outcome

Every verified finding must be reported as an individual GitHub issue in `upwell-rs/upwell-docs`. Do not merely return a list of gaps. A finding is only complete after either:

- a new issue has been created, or
- an existing issue is linked as the duplicate record.

Do not report speculative gaps. Confirm that the feature is public, supported in the selected release, and absent or materially inadequate in the deployed documentation before opening an issue.

## Sources of truth

Use the following evidence in this order:

1. The deployed release documentation, including guide navigation, guide pages, `/llms.txt`, `/sitemap.xml`, and generated symbol pages.
2. The matching framework release tag, normally `v<release>`, in `upwell-rs/upwell`. Use the checked-out workspace only when it is confirmed to be at that tag; otherwise inspect the tag remotely.
3. The documentation repository at `upwell-rs/upwell-docs`, especially `docs.config.ts`, `src/content/docs`, `src/content/symbols`, and generated documentation artifacts.
4. The local `manifest.yaml` beside this skill. It is the audit ledger and starting inventory, not proof that a page is correct.

Never use plans, TODOs, unreleased commits, or internal-only APIs as evidence of a release documentation gap.

## Audit workflow

1. Choose the release from the request. If none is specified, audit the release selected by the deployed site and record the exact release in the manifest.
2. Fetch the documentation home page and its machine-readable inventory (`/llms.txt` and `/sitemap.xml`). Record every guide URL, symbol page, and stated topic in the manifest.
3. Inspect the matching release's public surface:
   - facade crate features, modules, prelude exports, macros, and CLI;
   - public protocol crates and their feature-gated APIs;
   - configuration, lifecycle, DI, plugin, tooling, generated-client, and browser/wasm surfaces;
   - examples and public command behavior where they are part of the supported user workflow.
4. Compare each user-facing capability with the inventory. Account for discoverability through both a guide and generated symbol reference. A symbol page alone is not sufficient for a workflow, feature selection, operational behavior, compatibility constraint, or multi-step integration.
5. Score every capability using the quality model below. Update `manifest.yaml` with URLs, evidence, score, and audit date for both covered and uncovered capabilities.
6. Search all open and closed issues in `upwell-rs/upwell-docs` before creating each issue. Search title and body using the capability name, public API names, and release. Treat an issue as a duplicate only when it identifies the same documentation deliverable and release applicability.
7. Create one GitHub issue for every unique verified finding. Include the issue URL in the manifest and mark the finding `reported`.
8. Re-read the issue state after creation. If issue creation is unavailable because authentication or repository permissions are missing, stop and report the blocker explicitly. Do not claim the audit completed and do not leave findings unreported.

## Quality model

Score each capability from 0 through 4. The score measures documentation usefulness, not API maturity.

| Score | Meaning | Required evidence |
| --- | --- | --- |
| 0 | Undocumented | No guide or symbol reference that lets a user discover and use the public capability. |
| 1 | Mentioned only | A name, release note, or symbol exists, but no purpose, prerequisites, or usage guidance. |
| 2 | Basic reference | Purpose and API details exist, but feature activation, configuration, limitations, or a runnable minimal example is missing. |
| 3 | Usable guide | Explains purpose, prerequisites, activation/configuration, and a focused example; known constraints are stated. |
| 4 | Complete workflow | Score 3 plus integration/operational guidance, compatibility or feature interactions, and links to relevant symbol/reference material. |

Open an issue for scores 0-2. Open an issue for score 3 when omission of a safety, compatibility, migration, or operational constraint is likely to mislead users. Do not create quality issues solely to pursue stylistic preferences.

## Capability inventory

At minimum, audit these capability families. Extend the manifest when the public surface identifies another user-facing family.

- Getting started, supported Rust/toolchain versions, release compatibility, and feature selection.
- `app!`, direct `App::<Protocol>::builder`, application lifecycle, generated CLI, configuration, and shutdown.
- Components, providers, scopes/lifetimes, dependency validation, configuration injection, hooks, and plugins.
- Native RPC services, handler declarations, transports, errors, and generated clients.
- HTTP controllers, route handlers, DTOs, extractors, responses/streaming, multipart, and request extensions.
- WebSocket protocol authoring, JSON-over-WebSocket, STOMP topics/subscriptions, and relevant clients.
- Scheduled jobs, execution policy, observability, and lifecycle integration.
- OpenAPI generation, supported UI integrations, and configuration.
- Native and wasm/browser generated-client setup, transport/backend feature selection, and TypeScript integration.
- `cargo upwell`: initialization, probing, inspect/query/graph/export, automation, extensions, and template/catalog management.
- Tooling schema/export behavior, protocol composition, watch/reload, feature compatibility, and platform constraints.

## Issue format

Use the title format:

```text
docs(<area>): document <capability>
```

Use this body template. Keep source evidence specific and avoid proposed implementation details unless they are necessary acceptance criteria.

```markdown
## Gap

<What a release user cannot discover, configure, or use correctly.>

## Release and public surface

- Release: `<release>`
- Public API / command: `<fully qualified name, macro, feature, or command>`
- Framework evidence: <tag URL and source path or symbol URL>

## Documentation evidence

- Deployed documentation searched: <URLs>
- Current coverage: <why it is absent or inadequate>
- Quality score: <0-3>/4

## Requested documentation

- [ ] <Observable documentation outcome>
- [ ] <Example, prerequisite, constraint, or link required for correct use>

## Acceptance criteria

- A reader can determine when to use the capability.
- A reader can enable/configure it with the correct public API and features.
- A reader can follow a minimal correct example.
- Relevant limitations, compatibility constraints, and related guides are linked.
```

## Manifest rules

- Keep `manifest.yaml` versioned with the skill.
- Record precise URLs and release-tag source references, not general assertions.
- Use stable capability IDs such as `axum.openapi` or `tooling.export`.
- Preserve prior audit entries and issue URLs; update an entry when coverage changes rather than deleting evidence.
- A `reported` entry must contain exactly one of `issue_url` or `duplicate_issue_url`.
- Do not add GitHub tokens, credentials, local API payloads, or generated site copies to the manifest.

## Completion report

Report the audited release, number of capabilities checked, score distribution, all new issue URLs, all duplicate issue URLs, and blockers. The audit is incomplete if any verified finding lacks an issue or duplicate record.
