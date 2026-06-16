<!--
Sync Impact Report
Version change: N/A (template) → 1.0.0
Modified principles:
- PRINCIPLE_1_NAME placeholder → I. Spec-First Delivery
- PRINCIPLE_2_NAME placeholder → II. Minimal Scoped Changes
- PRINCIPLE_3_NAME placeholder → III. Testable Increments
- PRINCIPLE_4_NAME placeholder → IV. Explicit Interfaces and Data Contracts
- PRINCIPLE_5_NAME placeholder → V. Operational Traceability and Security
Added sections:
- Engineering Constraints
- Development Workflow and Quality Gates
Removed sections:
- None
Templates requiring updates:
- ✅ updated .specify/templates/plan-template.md
- ✅ updated .specify/templates/spec-template.md
- ✅ updated .specify/templates/tasks-template.md
- ✅ reviewed .specify/templates/checklist-template.md
- ✅ reviewed .specify/extensions/*/commands/*.md
- ✅ reviewed AGENTS.md
Follow-up TODOs: None
-->
# Overseer Constitution

## Core Principles

### I. Spec-First Delivery

Every feature MUST begin from a written specification that defines user value,
independent user journeys, acceptance scenarios, edge cases, requirements,
assumptions, and measurable success criteria. Implementation plans and task lists
MUST trace back to the approved specification. Work that changes behavior without
updating the relevant specification, plan, or task artifact is non-compliant.

Rationale: Overseer development depends on explicit intent so implementation,
review, and verification can stay aligned across sessions and contributors.

### II. Minimal Scoped Changes

Changes MUST be limited to files and behavior required by the current
specification or task. Broad refactors, unrelated cleanup, dependency churn, or
format-only rewrites MUST NOT be included unless they are explicitly called out
as in scope and justified in the plan. Any unavoidable cross-cutting change MUST
be documented in Complexity Tracking with the rejected simpler alternative.

Rationale: Small, targeted changes reduce regression risk and preserve reviewer
confidence in the relationship between a request and the resulting diff.

### III. Testable Increments

Each user story MUST be independently verifiable before dependent or lower
priority stories are considered complete. Plans MUST identify the verification
method for each story. Tasks MUST include automated tests when behavior can be
checked programmatically; otherwise they MUST include explicit manual validation
steps with expected results. Bug fixes MUST reproduce the failure before the fix
is accepted whenever reproduction is practical.

Rationale: Independent verification keeps MVP slices shippable and prevents
later work from masking incomplete or broken behavior.

### IV. Explicit Interfaces and Data Contracts

External interfaces, command behavior, data models, storage expectations, and
integration boundaries MUST be specified before implementation. Contract changes
MUST identify compatibility impact, migration needs, and affected consumers.
Implementation MUST NOT rely on hidden behavior that is absent from the feature
specification or plan.

Rationale: Explicit contracts make integration work reviewable and prevent
silent breaking changes across tools, services, and persisted data.

### V. Operational Traceability and Security

Features MUST define operationally relevant logging, error handling, observability,
configuration, and security expectations appropriate to their scope. Sensitive
data MUST NOT be logged or embedded in generated artifacts. User-facing failures
MUST provide actionable errors without exposing secrets. Any new persistence,
network, credential, or permission boundary MUST be reviewed in the plan.

Rationale: Overseer must remain diagnosable and safe when running unattended or
interacting with user data and external systems.

## Engineering Constraints

- Specifications MUST remain technology-agnostic until the implementation plan
  selects concrete technologies and explains why they fit the feature.
- Plans MUST document target platform, dependencies, storage, testing approach,
  performance goals, constraints, and scale/scope before implementation starts.
- Generated artifacts MUST keep exact file paths for planned changes and tasks.
- New dependencies MUST be justified by user value, maintenance cost, and a
  simpler alternative considered in the plan.
- Any behavior that affects existing data, storage formats, command interfaces,
  APIs, permissions, or configuration MUST include compatibility and migration
  notes before implementation.

## Development Workflow and Quality Gates

1. Specification comes first: capture user journeys, acceptance scenarios,
   requirements, assumptions, and measurable success criteria.
2. Planning follows the specification: resolve technical context, interface/data
   contracts, operational concerns, and constitution checks before tasks.
3. Tasks are grouped by independently testable user story and include verification
   work for each story before implementation is considered complete.
4. Implementation proceeds in priority order unless the plan explicitly justifies
   parallel work that does not create file or story conflicts.
5. Reviews MUST verify constitution compliance, traceability from spec to tasks,
   scoped diffs, and evidence that required verification passed.
6. Non-compliance MUST be recorded in Complexity Tracking with rationale and a
   simpler alternative that was rejected.

## Governance

This constitution supersedes conflicting project practices, templates, and
informal instructions. Amendments MUST update this file, include a Sync Impact
Report, propagate required changes to dependent templates and runtime guidance,
and record the semantic version change.

Versioning policy:
- MAJOR: Removes or redefines a core principle or governance rule in a backward
  incompatible way.
- MINOR: Adds a core principle, required section, quality gate, or materially
  expands mandatory guidance.
- PATCH: Clarifies wording, fixes errors, or makes non-semantic refinements.

Compliance review is required during planning and again during review of the
resulting changes. Any approved exception MUST be explicit, scoped, and tied to
a tracked feature or task.

**Version**: 1.0.0 | **Ratified**: 2026-06-16 | **Last Amended**: 2026-06-16
