<!-- SPECKIT START -->
For additional context about technologies to be used, project structure,
shell commands, and other important information, read the current plan
at specs/003-response-status-codes/plan.md
<!-- SPECKIT END -->

When you need to commit anything use `mise exec -- git <command>` in order to load the correct git profiles via mise.

## Before every commit / PR

- **Format.** Run `cargo fmt --all` before committing. A PR with unformatted code is blocked
  (`cargo fmt --all -- --check` must pass) — CI enforces it, so never push without it.
- **Lint.** `cargo clippy --workspace --all-targets --all-features` must be warning-free (this is
  the CI invocation — reproduce failures with it, not a per-crate clippy).

## Test layout

Test modules always live in their own file, never inline in an impl file. For a module `foo`
(`foo.rs` or `foo/mod.rs`), declare `#[cfg(test)] mod tests;` in the module and put the tests in a
sibling `foo/tests.rs` (a `foo.rs` file may keep its `foo/tests.rs` submodule without becoming
`foo/mod.rs`). This keeps impl files clean and avoids the `clippy::items_after_test_module` lint.
Most modules with tests therefore gain a `<module>/tests.rs` file. 