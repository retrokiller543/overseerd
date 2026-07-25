use std::collections::{BTreeMap, BTreeSet};

use super::diagnostic::{CompositionDiagnostic, CompositionEdge, CompositionTarget};
use super::resolver::ResolvedPlugin;
use super::{
    CompositionPhase, InstallationProvenance, PluginId, PluginSlotId, RelationKind, RelationTarget,
};

/// A deterministic directed plugin graph with attributed edge kinds.
pub(super) type Graph = BTreeMap<PluginId, BTreeMap<PluginId, BTreeSet<RelationKind>>>;

/// Validates effective relations and creates the graph for the active phase.
pub(super) fn validate(
    phase: CompositionPhase,
    plugins: &[ResolvedPlugin],
    prior: &[ResolvedPlugin],
    diagnostics: &mut Vec<CompositionDiagnostic>,
) -> Graph {
    let prior_by_id: BTreeMap<_, _> = prior.iter().map(|plugin| (plugin.id(), plugin)).collect();
    let prior_by_slot: BTreeMap<_, _> = prior
        .iter()
        .filter_map(|plugin| plugin.slot().map(|(slot, _)| (slot, plugin)))
        .collect();
    let current_by_id: BTreeMap<_, _> =
        plugins.iter().map(|plugin| (plugin.id(), plugin)).collect();
    let current_by_slot: BTreeMap<_, _> = plugins
        .iter()
        .filter_map(|plugin| plugin.slot().map(|(slot, _)| (slot, plugin)))
        .collect();
    let indexes = Indexes {
        prior_by_id: &prior_by_id,
        prior_by_slot: &prior_by_slot,
        current_by_id: &current_by_id,
        current_by_slot: &current_by_slot,
    };
    let mut graph = Graph::new();
    let mut conflicts = BTreeSet::new();

    if phase == CompositionPhase::Late {
        validate_prior_relations(&indexes, &mut conflicts, diagnostics);
    }

    for plugin in plugins {
        graph.entry(plugin.id()).or_default();

        for relation in plugin.relations() {
            validate_relation(
                phase,
                plugin,
                relation.kind(),
                relation.target(),
                &indexes,
                &mut graph,
                &mut conflicts,
                diagnostics,
            );
        }
    }

    report_conflicts(&indexes, conflicts, diagnostics);

    graph
}

/// Produces canonical cycle diagnostics for every cyclic component.
pub(super) fn cycles(
    phase: CompositionPhase,
    plugins: &[ResolvedPlugin],
    graph: &Graph,
) -> Vec<CompositionDiagnostic> {
    let unresolved = unresolved_nodes(plugins, graph);
    let provenance: BTreeMap<_, _> = plugins
        .iter()
        .map(|plugin| (plugin.id(), plugin.provenance()))
        .collect();

    cycle_diagnostics(phase, graph, &unresolved, &provenance)
}

/// Orders an acyclic graph using lexical plugin identity as the ready-node tie-breaker.
pub(super) fn order(plugins: Vec<ResolvedPlugin>, graph: &Graph) -> Vec<ResolvedPlugin> {
    let mut by_id: BTreeMap<_, _> = plugins
        .into_iter()
        .map(|plugin| (plugin.id(), plugin))
        .collect();
    let mut indegree = indegrees(by_id.keys().copied(), graph);
    let mut ready = ready_nodes(&indegree);
    let mut order = Vec::with_capacity(by_id.len());

    while let Some(next) = ready.pop_first() {
        order.push(next);
        release_successors(next, graph, &mut indegree, &mut ready);
    }

    order
        .into_iter()
        .map(|id| by_id.remove(&id).expect("ordered plugin exists"))
        .collect()
}

struct Indexes<'a> {
    prior_by_id: &'a BTreeMap<PluginId, &'a ResolvedPlugin>,
    prior_by_slot: &'a BTreeMap<PluginSlotId, &'a ResolvedPlugin>,
    current_by_id: &'a BTreeMap<PluginId, &'a ResolvedPlugin>,
    current_by_slot: &'a BTreeMap<PluginSlotId, &'a ResolvedPlugin>,
}

struct Target<'a> {
    plugin: &'a ResolvedPlugin,
    is_prior: bool,
}

impl Target<'_> {
    fn id(&self) -> PluginId {
        self.plugin.id()
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_relation(
    phase: CompositionPhase,
    plugin: &ResolvedPlugin,
    kind: RelationKind,
    declared_target: RelationTarget,
    indexes: &Indexes<'_>,
    graph: &mut Graph,
    conflicts: &mut BTreeSet<(PluginId, PluginId)>,
    diagnostics: &mut Vec<CompositionDiagnostic>,
) {
    let Some(target) = resolve_target(declared_target, indexes) else {
        if kind == RelationKind::Requires {
            diagnostics.push(CompositionDiagnostic::MissingDependency {
                plugin: plugin.id(),
                provenance: plugin.provenance(),
                target: declared_target,
            });
        }

        return;
    };

    if target.id() == plugin.id() {
        diagnostics.push(self_relation(plugin, kind, declared_target));

        return;
    }

    match kind {
        RelationKind::Conflicts => {
            conflicts.insert(ordered_pair(plugin.id(), target.id()));
        }
        RelationKind::Before if phase == CompositionPhase::Late && target.is_prior => {
            diagnostics.push(CompositionDiagnostic::LateOrderingBeforeEarly {
                plugin: plugin.id(),
                target: target.id(),
                provenance: plugin.provenance(),
            });
        }
        RelationKind::Requires | RelationKind::After if target.is_prior => {}
        RelationKind::Requires => {
            insert_edge(graph, target.id(), plugin.id(), RelationKind::Requires);
        }
        RelationKind::Before => {
            insert_edge(graph, plugin.id(), target.id(), RelationKind::Before);
        }
        RelationKind::After => {
            insert_edge(graph, target.id(), plugin.id(), RelationKind::After);
        }
    }
}

fn self_relation(
    plugin: &ResolvedPlugin,
    kind: RelationKind,
    target: RelationTarget,
) -> CompositionDiagnostic {
    match kind {
        RelationKind::Requires => CompositionDiagnostic::SelfDependency {
            plugin: plugin.id(),
            provenance: plugin.provenance(),
            target,
        },
        RelationKind::Conflicts => CompositionDiagnostic::SelfConflict {
            plugin: plugin.id(),
            provenance: plugin.provenance(),
            target,
        },
        RelationKind::Before | RelationKind::After => CompositionDiagnostic::SelfOrdering {
            plugin: plugin.id(),
            provenance: plugin.provenance(),
            kind,
            target,
        },
    }
}

fn validate_prior_relations(
    indexes: &Indexes<'_>,
    conflicts: &mut BTreeSet<(PluginId, PluginId)>,
    diagnostics: &mut Vec<CompositionDiagnostic>,
) {
    for plugin in indexes.prior_by_id.values() {
        for relation in plugin.relations() {
            let Some(target) = resolve_target(relation.target(), indexes) else {
                continue;
            };

            if target.id() == plugin.id() {
                continue;
            }

            match relation.kind() {
                RelationKind::Conflicts => {
                    conflicts.insert(ordered_pair(plugin.id(), target.id()));
                }
                RelationKind::After if !target.is_prior => {
                    diagnostics.push(CompositionDiagnostic::EarlyOrderingAfterLate {
                        plugin: plugin.id(),
                        target: target.id(),
                        provenance: plugin.provenance(),
                    });
                }
                RelationKind::Requires | RelationKind::Before | RelationKind::After => {}
            }
        }
    }
}

fn resolve_target<'a>(target: RelationTarget, indexes: &'a Indexes<'a>) -> Option<Target<'a>> {
    match target {
        RelationTarget::Plugin(id) => indexes
            .current_by_id
            .get(&id)
            .map(|plugin| Target {
                plugin,
                is_prior: false,
            })
            .or_else(|| {
                indexes.prior_by_id.get(&id).map(|plugin| Target {
                    plugin,
                    is_prior: true,
                })
            }),
        RelationTarget::Slot(slot) => indexes
            .current_by_slot
            .get(&slot)
            .map(|plugin| Target {
                plugin,
                is_prior: false,
            })
            .or_else(|| {
                indexes.prior_by_slot.get(&slot).map(|plugin| Target {
                    plugin,
                    is_prior: true,
                })
            }),
    }
}

fn report_conflicts(
    indexes: &Indexes<'_>,
    conflicts: BTreeSet<(PluginId, PluginId)>,
    diagnostics: &mut Vec<CompositionDiagnostic>,
) {
    for (left, right) in conflicts {
        let left_plugin = plugin_by_id(left, indexes);
        let right_plugin = plugin_by_id(right, indexes);

        diagnostics.push(CompositionDiagnostic::Conflict {
            left,
            left_provenance: left_plugin.provenance(),
            right,
            right_provenance: right_plugin.provenance(),
        });
    }
}

fn plugin_by_id<'a>(id: PluginId, indexes: &'a Indexes<'a>) -> &'a ResolvedPlugin {
    indexes
        .current_by_id
        .get(&id)
        .or_else(|| indexes.prior_by_id.get(&id))
        .copied()
        .expect("resolved relation target exists")
}

fn ordered_pair(left: PluginId, right: PluginId) -> (PluginId, PluginId) {
    if left < right {
        (left, right)
    } else {
        (right, left)
    }
}

fn insert_edge(graph: &mut Graph, from: PluginId, to: PluginId, kind: RelationKind) {
    graph
        .entry(from)
        .or_default()
        .entry(to)
        .or_default()
        .insert(kind);
    graph.entry(to).or_default();
}

fn unresolved_nodes(plugins: &[ResolvedPlugin], graph: &Graph) -> BTreeSet<PluginId> {
    let mut indegree = indegrees(plugins.iter().map(ResolvedPlugin::id), graph);
    let mut ready = ready_nodes(&indegree);

    while let Some(next) = ready.pop_first() {
        release_successors(next, graph, &mut indegree, &mut ready);
    }

    indegree
        .into_iter()
        .filter_map(|(id, degree)| (degree > 0).then_some(id))
        .collect()
}

fn indegrees(
    nodes: impl IntoIterator<Item = PluginId>,
    graph: &Graph,
) -> BTreeMap<PluginId, usize> {
    let mut indegree: BTreeMap<_, usize> = nodes.into_iter().map(|id| (id, 0)).collect();

    for successors in graph.values() {
        for successor in successors.keys() {
            *indegree.get_mut(successor).expect("graph node exists") += 1;
        }
    }

    indegree
}

fn ready_nodes(indegree: &BTreeMap<PluginId, usize>) -> BTreeSet<PluginId> {
    indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect()
}

fn release_successors(
    node: PluginId,
    graph: &Graph,
    indegree: &mut BTreeMap<PluginId, usize>,
    ready: &mut BTreeSet<PluginId>,
) {
    if let Some(successors) = graph.get(&node) {
        for successor in successors.keys() {
            let degree = indegree.get_mut(successor).expect("graph node exists");

            *degree -= 1;

            if *degree == 0 {
                ready.insert(*successor);
            }
        }
    }
}

fn cycle_diagnostics(
    phase: CompositionPhase,
    graph: &Graph,
    unresolved: &BTreeSet<PluginId>,
    provenance: &BTreeMap<PluginId, InstallationProvenance>,
) -> Vec<CompositionDiagnostic> {
    let components = strongly_connected_components(graph, unresolved);
    let mut diagnostics = Vec::new();

    for component in components {
        if component.len() == 1 {
            let id = component[0];
            let has_self_edge = graph
                .get(&id)
                .is_some_and(|successors| successors.contains_key(&id));

            if !has_self_edge {
                continue;
            }
        }

        let members: Box<_> = component
            .iter()
            .map(|id| CompositionTarget::new(*id, provenance[id]))
            .collect();
        let steps = representative_cycle(graph, &component, provenance);
        let display = display_cycle(&steps);

        diagnostics.push(CompositionDiagnostic::Cycle {
            phase,
            members,
            steps: steps.into_boxed_slice(),
            display,
        });
    }

    diagnostics
}

fn strongly_connected_components(graph: &Graph, nodes: &BTreeSet<PluginId>) -> Vec<Vec<PluginId>> {
    let mut state = TarjanState::default();

    for node in nodes {
        if !state.indices.contains_key(node) {
            visit(*node, graph, nodes, &mut state);
        }
    }

    state.components.sort_by_key(|component| component[0]);

    state.components
}

#[derive(Default)]
struct TarjanState {
    next_index: usize,
    indices: BTreeMap<PluginId, usize>,
    lowlinks: BTreeMap<PluginId, usize>,
    stack: Vec<PluginId>,
    on_stack: BTreeSet<PluginId>,
    components: Vec<Vec<PluginId>>,
}

fn visit(node: PluginId, graph: &Graph, nodes: &BTreeSet<PluginId>, state: &mut TarjanState) {
    let index = state.next_index;

    state.next_index += 1;
    state.indices.insert(node, index);
    state.lowlinks.insert(node, index);
    state.stack.push(node);
    state.on_stack.insert(node);

    for successor in graph.get(&node).into_iter().flat_map(|items| items.keys()) {
        if !nodes.contains(successor) {
            continue;
        }

        if !state.indices.contains_key(successor) {
            visit(*successor, graph, nodes, state);
            let successor_lowlink = state.lowlinks[successor];
            let node_lowlink = state.lowlinks[&node];

            state
                .lowlinks
                .insert(node, node_lowlink.min(successor_lowlink));
        } else if state.on_stack.contains(successor) {
            let successor_index = state.indices[successor];
            let node_lowlink = state.lowlinks[&node];

            state
                .lowlinks
                .insert(node, node_lowlink.min(successor_index));
        }
    }

    if state.lowlinks[&node] == state.indices[&node] {
        finish_component(node, state);
    }
}

fn finish_component(node: PluginId, state: &mut TarjanState) {
    let mut component = Vec::new();

    loop {
        let member = state.stack.pop().expect("active SCC node exists");

        state.on_stack.remove(&member);
        component.push(member);

        if member == node {
            break;
        }
    }

    component.sort();
    state.components.push(component);
}

fn representative_cycle(
    graph: &Graph,
    component: &[PluginId],
    provenance: &BTreeMap<PluginId, InstallationProvenance>,
) -> Vec<CompositionEdge> {
    let start = component[0];
    let members: BTreeSet<_> = component.iter().copied().collect();
    let mut visited = BTreeSet::from([start]);
    let mut path = Vec::new();

    let found = search_cycle(start, start, graph, &members, &mut visited, &mut path);

    assert!(found, "strongly connected component contains a cycle");

    path.into_iter()
        .map(|(from, to)| {
            let kinds = graph[&from][&to].iter().copied().collect();

            CompositionEdge::new(
                CompositionTarget::new(from, provenance[&from]),
                CompositionTarget::new(to, provenance[&to]),
                kinds,
            )
        })
        .collect()
}

fn search_cycle(
    current: PluginId,
    start: PluginId,
    graph: &Graph,
    members: &BTreeSet<PluginId>,
    visited: &mut BTreeSet<PluginId>,
    path: &mut Vec<(PluginId, PluginId)>,
) -> bool {
    for successor in graph
        .get(&current)
        .into_iter()
        .flat_map(|items| items.keys())
        .filter(|id| members.contains(id))
    {
        if *successor == start {
            path.push((current, start));

            return true;
        }

        if !visited.insert(*successor) {
            continue;
        }

        path.push((current, *successor));

        if search_cycle(*successor, start, graph, members, visited, path) {
            return true;
        }

        path.pop();
    }

    false
}

fn display_cycle(steps: &[CompositionEdge]) -> String {
    let mut display = String::new();

    if let Some(first) = steps.first() {
        display.push_str(first.from().plugin().as_str());

        for step in steps {
            display.push_str(" -> ");
            display.push_str(step.to().plugin().as_str());
        }
    }

    display
}
