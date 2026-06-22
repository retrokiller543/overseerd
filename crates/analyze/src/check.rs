//! Graph checks over the parsed [`Model`]: the build-time mirror of the runtime
//! `DescriptorRegistry::validate` checks that can be decided from source. v1
//! covers missing dependencies and dependency cycles; ambiguous providers,
//! duplicate ids, and duplicate RPC paths are planned next.

use std::collections::{HashMap, HashSet};

use crate::Diagnostic;
use crate::parse::Model;

/// Runs every check and returns the collected diagnostics (empty = valid).
pub fn run(model: &Model) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    missing_dependencies(model, &mut diagnostics);
    dependency_cycles(model, &mut diagnostics);

    diagnostics
}

/// A required concrete dependency whose type carries no `Component`-ish attribute
/// anywhere in the crate — and is not marked `Dynamic` — cannot be satisfied.
fn missing_dependencies(model: &Model, diagnostics: &mut Vec<Diagnostic>) {
    let known: HashSet<&str> = model.components.iter().map(|c| c.name.as_str()).collect();

    for component in &model.components {
        for dep in &component.deps {
            if !known.contains(dep.name.as_str()) {
                diagnostics.push(Diagnostic {
                    file: component.file.clone(),
                    line: dep.line,
                    message: format!(
                        "no component provides `{}`, required by `{}` — add a component \
                         for it, or mark the dependency `Dynamic` if it is provided at runtime",
                        dep.name, component.name
                    ),
                });
            }
        }
    }
}

/// Detects cycles in the component dependency graph (edges to deps that resolve
/// to a known component). A cycle has no valid construction order.
fn dependency_cycles(model: &Model, diagnostics: &mut Vec<Diagnostic>) {
    let known: HashSet<&str> = model.components.iter().map(|c| c.name.as_str()).collect();

    let edges: HashMap<&str, Vec<&str>> = model
        .components
        .iter()
        .map(|c| {
            let deps = c
                .deps
                .iter()
                .map(|d| d.name.as_str())
                .filter(|name| known.contains(name))
                .collect();

            (c.name.as_str(), deps)
        })
        .collect();

    let mut visited: HashSet<&str> = HashSet::new();
    let mut on_stack: HashSet<&str> = HashSet::new();
    let mut reported: HashSet<&str> = HashSet::new();

    for component in &model.components {
        visit(
            component.name.as_str(),
            &edges,
            &mut visited,
            &mut on_stack,
            &mut reported,
            model,
            diagnostics,
        );
    }
}

/// Depth-first cycle search. On a back-edge, reports the node that closes the
/// cycle (deduplicated, so each cyclic component is named at most once).
fn visit<'a>(
    node: &'a str,
    edges: &HashMap<&'a str, Vec<&'a str>>,
    visited: &mut HashSet<&'a str>,
    on_stack: &mut HashSet<&'a str>,
    reported: &mut HashSet<&'a str>,
    model: &Model,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if visited.contains(node) {
        return;
    }

    visited.insert(node);
    on_stack.insert(node);

    for &next in edges.get(node).into_iter().flatten() {
        if on_stack.contains(next) {
            if reported.insert(next) {
                let component = model.components.iter().find(|c| c.name == next);

                if let Some(component) = component {
                    diagnostics.push(Diagnostic {
                        file: component.file.clone(),
                        line: component.line,
                        message: format!(
                            "dependency cycle through `{next}` — components cannot depend on \
                             themselves transitively"
                        ),
                    });
                }
            }
        } else {
            visit(next, edges, visited, on_stack, reported, model, diagnostics);
        }
    }

    on_stack.remove(node);
}