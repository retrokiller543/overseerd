//! Build-time validation of an Overseer dependency graph.
//!
//! Call [`validate_crate`] from a `build.rs` to fail `cargo build` (in CI,
//! before deployment) on dependency-graph errors that the runtime would
//! otherwise only catch at `daemon.build()`:
//!
//! ```ignore
//! // build.rs
//! fn main() {
//!     overseer_analyze::report(overseer_analyze::validate_crate("src"));
//! }
//! ```
//!
//! It parses the crate's sources with `syn` and reconstructs the graph by name.
//! This is **intra-crate and name-based** (see [`parse`]): a dependency provided
//! at runtime must live in the same crate (carrying a `Component`-ish attribute)
//! or be marked `Dynamic`. Cross-crate providers are not visible here.

use std::path::{Path, PathBuf};

mod check;
mod parse;

/// One validation error, located in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: PathBuf,
    pub line: usize,
    pub message: String,
}

/// Validates the dependency graph reconstructed from the `.rs` files under
/// `src_dir`. `Ok(())` when the graph is sound, otherwise the collected errors.
pub fn validate_crate(src_dir: impl AsRef<Path>) -> Result<(), Vec<Diagnostic>> {
    let model = parse::Model::from_dir(src_dir.as_ref());

    finish(check::run(&model))
}

/// Validates a single source string. Useful for tests and for callers that
/// assemble their own file set.
pub fn validate_source(source: &str) -> Result<(), Vec<Diagnostic>> {
    let mut model = parse::Model::default();

    model.add_source(source, Path::new("<source>"));

    finish(check::run(&model))
}

fn finish(diagnostics: Vec<Diagnostic>) -> Result<(), Vec<Diagnostic>> {
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

/// Reports a validation result to Cargo from a `build.rs`: prints each error via
/// `cargo::error=` and panics so the build fails. A no-op on success.
pub fn report(result: Result<(), Vec<Diagnostic>>) {
    let Err(diagnostics) = result else {
        return;
    };

    for diagnostic in &diagnostics {
        println!(
            "cargo::error={}:{}: {}",
            diagnostic.file.display(),
            diagnostic.line,
            diagnostic.message
        );
    }

    panic!(
        "overseer: dependency validation failed with {} error(s)",
        diagnostics.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_satisfied_graph() {
        let source = r#"
            #[derive(Component)]
            struct Config { url: String }

            #[component]
            struct Pool { config: Arc<Config>, #[default] hits: u32 }

            #[service]
            struct Api { pool: Arc<Pool> }
        "#;

        assert!(validate_source(source).is_ok());
    }

    #[test]
    fn flags_a_missing_dependency() {
        let source = r#"
            #[component]
            struct Pool { config: Arc<Config> }
        "#;

        let errors = validate_source(source).unwrap_err();

        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("Config"));
        assert!(errors[0].message.contains("Pool"));
    }

    #[test]
    fn default_and_dynamic_fields_are_not_missing() {
        let source = r#"
            #[component]
            struct Thing {
                #[default]
                count: u32,
                cfg: Dynamic<Arc<Settings>>,
                cache: Option<Arc<Cache>>,
            }
        "#;

        // `#[default]` is local state; `Dynamic`/`Option` are not required-concrete.
        assert!(validate_source(source).is_ok());
    }

    #[test]
    fn detects_a_dependency_cycle() {
        let source = r#"
            #[component]
            struct A { b: Arc<B> }

            #[component]
            struct B { a: Arc<A> }
        "#;

        let errors = validate_source(source).unwrap_err();

        assert!(
            errors.iter().any(|e| e.message.contains("cycle")),
            "expected a cycle diagnostic, got: {errors:?}"
        );
    }
}