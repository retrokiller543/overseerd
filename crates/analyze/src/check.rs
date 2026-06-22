//! Graph checks over the parsed [`Model`] — the build-time mirror of the runtime
//! `DescriptorRegistry::validate`: missing dependencies (concrete or trait),
//! ambiguous providers, qualifier resolution, duplicate component/service ids,
//! duplicate RPC paths, and dependency cycles.

use std::collections::{HashMap, HashSet};

use crate::Diagnostic;
use crate::parse::{Dependency, Model, Struct};

/// A single provider entry, flattened from the structs' `provide = ..`.
struct Provider<'a> {
    trait_name: &'a str,
    concrete: &'a str,
    qualifier: &'a str,
    primary: bool,
}

/// Per-trait provider summary used by the missing/ambiguity checks.
#[derive(Default)]
struct TraitProviders<'a> {
    total: usize,
    primary: usize,
    qualifiers: Vec<&'a str>,
    concretes: Vec<&'a str>,
}

/// Runs every check and returns the collected diagnostics (empty = valid).
pub fn run(model: &Model) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    let known: HashSet<&str> = model.structs.iter().map(|s| s.name.as_str()).collect();
    let providers = collect_providers(model);
    let by_trait = group_providers(&providers);
    let resolved = resolved_deps(model);

    missing_dependencies(&resolved, &known, &by_trait, &mut diagnostics);
    ambiguous_providers(&resolved, &by_trait, &mut diagnostics);
    duplicate_ids(model, &mut diagnostics);
    duplicate_rpc_paths(model, &mut diagnostics);
    dependency_cycles(&resolved, &known, &by_trait, &mut diagnostics);

    diagnostics
}

/// An injected component paired with the dependencies it actually constructs
/// from: an `#[init]` block's parameters override field injection.
struct Resolved<'a> {
    component: &'a Struct,
    deps: &'a [Dependency],
}

fn resolved_deps(model: &Model) -> Vec<Resolved<'_>> {
    model
        .structs
        .iter()
        .filter(|s| s.injected)
        .map(|s| {
            let init = model
                .impls
                .iter()
                .find(|h| h.self_name == s.name)
                .and_then(|h| h.init_deps.as_deref());

            Resolved {
                component: s,
                deps: init.unwrap_or(&s.field_deps),
            }
        })
        .collect()
}

fn collect_providers(model: &Model) -> Vec<Provider<'_>> {
    model
        .structs
        .iter()
        .flat_map(|s| {
            s.provides.iter().map(move |p| Provider {
                trait_name: &p.trait_name,
                concrete: &s.name,
                qualifier: &p.qualifier,
                primary: p.primary,
            })
        })
        .collect()
}

fn group_providers<'a>(providers: &'a [Provider<'a>]) -> HashMap<&'a str, TraitProviders<'a>> {
    let mut by_trait: HashMap<&str, TraitProviders> = HashMap::new();

    for provider in providers {
        let entry = by_trait.entry(provider.trait_name).or_default();
        entry.total += 1;
        entry.primary += usize::from(provider.primary);
        entry.qualifiers.push(provider.qualifier);
        entry.concretes.push(provider.concrete);
    }

    by_trait
}

/// A required dependency with no satisfier: a concrete type that is not a known
/// component, or a trait with no (matching-qualifier) provider.
fn missing_dependencies(
    resolved: &[Resolved],
    known: &HashSet<&str>,
    by_trait: &HashMap<&str, TraitProviders>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for item in resolved {
        for dep in item.deps {
            let satisfied = if dep.is_trait {
                match (&dep.qualifier, by_trait.get(dep.name.as_str())) {
                    (Some(q), Some(providers)) => providers.qualifiers.contains(&q.as_str()),
                    (None, Some(_)) => true,
                    (_, None) => false,
                }
            } else {
                known.contains(dep.name.as_str())
            };

            if !satisfied {
                diagnostics.push(missing_diagnostic(item.component, dep));
            }
        }
    }
}

fn missing_diagnostic(component: &Struct, dep: &Dependency) -> Diagnostic {
    let what = match &dep.qualifier {
        Some(q) => format!("`{}` with qualifier `{q}`", dep.name),
        None => format!("`{}`", dep.name),
    };
    let hint = if dep.is_trait {
        "add a component with `provide = dyn ..`, or mark the dependency `Dynamic`"
    } else {
        "add a component for it, or mark the dependency `Dynamic` if provided at runtime"
    };

    Diagnostic {
        file: component.file.clone(),
        line: dep.line,
        message: format!(
            "nothing provides {what}, required by `{}` — {hint}",
            component.name
        ),
    }
}

/// A single `Arc<dyn Trait>` edge (no qualifier) with several providers needs a
/// unique `#[primary]`.
fn ambiguous_providers(
    resolved: &[Resolved],
    by_trait: &HashMap<&str, TraitProviders>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for item in resolved {
        for dep in item.deps {
            if !dep.is_trait || dep.qualifier.is_some() {
                continue;
            }

            if let Some(providers) = by_trait.get(dep.name.as_str())
                && providers.total > 1
                && providers.primary != 1
            {
                diagnostics.push(Diagnostic {
                    file: item.component.file.clone(),
                    line: dep.line,
                    message: format!(
                        "ambiguous provider for `{}`, required by `{}`: {} providers and no \
                         unique `#[primary]` — mark one `#[primary]`, select one with \
                         `#[qualifier]`, or inject `Vec`/`HashMap`",
                        dep.name, item.component.name, providers.total
                    ),
                });
            }
        }
    }
}

fn duplicate_ids(model: &Model, diagnostics: &mut Vec<Diagnostic>) {
    let mut component_ids: HashSet<&str> = HashSet::new();
    let mut service_ids: HashSet<&str> = HashSet::new();

    for item in &model.structs {
        if item.injected && !component_ids.insert(&item.id) {
            diagnostics.push(Diagnostic {
                file: item.file.clone(),
                line: item.line,
                message: format!("duplicate component id `{}`", item.id),
            });
        }

        if item.is_service && !service_ids.insert(&item.id) {
            diagnostics.push(Diagnostic {
                file: item.file.clone(),
                line: item.line,
                message: format!("duplicate service id `{}`", item.id),
            });
        }
    }
}

/// Two RPCs sharing a `Service.method` path collide on the wire.
fn duplicate_rpc_paths(model: &Model, diagnostics: &mut Vec<Diagnostic>) {
    let display: HashMap<&str, &str> = model
        .structs
        .iter()
        .filter(|s| s.is_service)
        .map(|s| (s.name.as_str(), s.display_name.as_str()))
        .collect();

    let mut seen: HashSet<String> = HashSet::new();

    for group in &model.impls {
        let service = display.get(group.self_name.as_str()).copied().unwrap_or(group.self_name.as_str());

        for rpc in &group.rpcs {
            let path = format!("{service}.{}", rpc.name);

            if !seen.insert(path.clone()) {
                diagnostics.push(Diagnostic {
                    file: group.file.clone(),
                    line: rpc.line,
                    message: format!("duplicate RPC path `{path}`"),
                });
            }
        }
    }
}

/// Detects cycles in the dependency graph (concrete edges to the dep's component,
/// trait edges to every provider's concrete).
fn dependency_cycles(
    resolved: &[Resolved],
    known: &HashSet<&str>,
    by_trait: &HashMap<&str, TraitProviders>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut location: HashMap<&str, (&std::path::Path, usize)> = HashMap::new();

    for item in resolved {
        location.insert(&item.component.name, (&item.component.file, item.component.line));
        let targets = edges.entry(&item.component.name).or_default();

        for dep in item.deps {
            if dep.is_trait {
                if let Some(providers) = by_trait.get(dep.name.as_str()) {
                    targets.extend(providers.concretes.iter().copied());
                }
            } else if known.contains(dep.name.as_str()) {
                targets.push(&dep.name);
            }
        }
    }

    let mut visited = HashSet::new();
    let mut on_stack = HashSet::new();
    let mut reported = HashSet::new();

    let nodes: Vec<&str> = edges.keys().copied().collect();

    for node in nodes {
        visit(
            node,
            &edges,
            &location,
            &mut visited,
            &mut on_stack,
            &mut reported,
            diagnostics,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn visit<'a>(
    node: &'a str,
    edges: &HashMap<&'a str, Vec<&'a str>>,
    location: &HashMap<&'a str, (&'a std::path::Path, usize)>,
    visited: &mut HashSet<&'a str>,
    on_stack: &mut HashSet<&'a str>,
    reported: &mut HashSet<&'a str>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !visited.insert(node) {
        return;
    }

    on_stack.insert(node);

    for &next in edges.get(node).into_iter().flatten() {
        if on_stack.contains(next) {
            if reported.insert(next)
                && let Some(&(file, line)) = location.get(next)
            {
                diagnostics.push(Diagnostic {
                    file: file.to_path_buf(),
                    line,
                    message: format!(
                        "dependency cycle through `{next}` — components cannot depend on \
                         themselves transitively"
                    ),
                });
            }
        } else {
            visit(next, edges, location, visited, on_stack, reported, diagnostics);
        }
    }

    on_stack.remove(node);
}
