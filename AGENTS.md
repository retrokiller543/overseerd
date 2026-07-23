<!-- SPECKIT START -->
For additional context about technologies to be used, project structure,
shell commands, and other important information, read the current plan
at specs/003-response-status-codes/plan.md
<!-- SPECKIT END -->

When you need to commit anything use `mise exec -- git <command>` in order to load the correct git profiles via mise.

## GitHub delivery workflow

- **PR ownership.** The project owner reviews, approves, and merges pull requests. Agents must never
  merge a pull request, even when CI and automated review are green.
- **Completion gate.** Do not consider a feature or issue complete until automated review explicitly
  reports that no issues remain.
- **Review conversations.** After addressing a review finding, resolve its GitHub review conversation.
- **Early tracking PRs.** When starting an epic, immediately create its branch and tracking pull
  request so progress and automated review are visible as work is added. Apply the same workflow to a
  `release/<version>` branch once it has a diff to its base branch.
- **Branch names.** Prefer `<branch-type>/<epic-or-module>/<issue-or-feature>/<optional-additional-path>`.
  Examples: `feat/app-cli/143/lifecycle` and `fix/app-macro/158/fallible-builder`.

## Before every commit / PR

- **Format.** Run `cargo fmt --all` before committing. A PR with unformatted code is blocked
  (`cargo fmt --all -- --check` must pass) — CI enforces it, so never push without it.
- **Lint.** `cargo clippy --workspace --all-targets --all-features` must be warning-free (this is
  the CI invocation — reproduce failures with it, not a per-crate clippy).
- **Test.** Run test suites with cargo-nextest (`cargo nextest run`) rather than `cargo test`.
  Formatting, compilation checks, doctests, and Clippy continue to use their dedicated Cargo
  commands.

## Test layout

Test modules always live in their own file, never inline in an impl file. For a module `foo`
(`foo.rs` or `foo/mod.rs`), declare `#[cfg(test)] mod tests;` in the module and put the tests in a
sibling `foo/tests.rs` (a `foo.rs` file may keep its `foo/tests.rs` submodule without becoming
`foo/mod.rs`). This keeps impl files clean and avoids the `clippy::items_after_test_module` lint.
Most modules with tests therefore gain a `<module>/tests.rs` file. 
