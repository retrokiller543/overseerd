use std::collections::{BTreeMap, BTreeSet, VecDeque};

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
    let mut visited = BTreeSet::new();
    let mut finish_order = Vec::with_capacity(nodes.len());

    for node in nodes {
        if visited.insert(*node) {
            collect_finish_order(*node, graph, nodes, &mut visited, &mut finish_order);
        }
    }

    let reverse = reverse_graph(graph, nodes);
    let mut assigned = BTreeSet::new();
    let mut components = Vec::new();

    for node in finish_order.into_iter().rev() {
        if !assigned.insert(node) {
            continue;
        }

        let mut component = collect_component(node, &reverse, &mut assigned);

        component.sort();
        components.push(component);
    }

    components.sort_by_key(|component| component[0]);

    components
}

fn collect_finish_order(
    start: PluginId,
    graph: &Graph,
    nodes: &BTreeSet<PluginId>,
    visited: &mut BTreeSet<PluginId>,
    finish_order: &mut Vec<PluginId>,
) {
    let mut stack = vec![(start, successors(start, graph, nodes), 0)];

    while let Some((node, adjacent, index)) = stack.last_mut() {
        if let Some(successor) = adjacent.get(*index).copied() {
            *index += 1;

            if visited.insert(successor) {
                stack.push((successor, successors(successor, graph, nodes), 0));
            }

            continue;
        }

        finish_order.push(*node);
        stack.pop();
    }
}

fn successors(node: PluginId, graph: &Graph, nodes: &BTreeSet<PluginId>) -> Vec<PluginId> {
    graph
        .get(&node)
        .into_iter()
        .flat_map(|items| items.keys())
        .filter(|successor| nodes.contains(successor))
        .copied()
        .collect()
}

fn reverse_graph(
    graph: &Graph,
    nodes: &BTreeSet<PluginId>,
) -> BTreeMap<PluginId, BTreeSet<PluginId>> {
    let mut reverse: BTreeMap<_, BTreeSet<_>> =
        nodes.iter().map(|node| (*node, BTreeSet::new())).collect();

    for (source, destinations) in graph {
        if !nodes.contains(source) {
            continue;
        }

        for destination in destinations.keys().filter(|node| nodes.contains(node)) {
            reverse
                .get_mut(destination)
                .expect("reverse graph node exists")
                .insert(*source);
        }
    }

    reverse
}

fn collect_component(
    start: PluginId,
    reverse: &BTreeMap<PluginId, BTreeSet<PluginId>>,
    assigned: &mut BTreeSet<PluginId>,
) -> Vec<PluginId> {
    let mut component = Vec::new();
    let mut stack = vec![start];

    while let Some(node) = stack.pop() {
        component.push(node);

        for predecessor in reverse[&node].iter().rev() {
            if assigned.insert(*predecessor) {
                stack.push(*predecessor);
            }
        }
    }

    component
}

fn representative_cycle(
    graph: &Graph,
    component: &[PluginId],
    provenance: &BTreeMap<PluginId, InstallationProvenance>,
) -> Vec<CompositionEdge> {
    let start = component[0];
    let members: BTreeSet<_> = component.iter().copied().collect();
    let path = cycle_path(start, graph, &members);

    path.windows(2)
        .map(|edge| {
            let from = edge[0];
            let to = edge[1];
            let kinds = graph[&from][&to].iter().copied().collect();

            CompositionEdge::new(
                CompositionTarget::new(from, provenance[&from]),
                CompositionTarget::new(to, provenance[&to]),
                kinds,
            )
        })
        .collect()
}

fn cycle_path(start: PluginId, graph: &Graph, members: &BTreeSet<PluginId>) -> Vec<PluginId> {
    for successor in successors(start, graph, members) {
        if successor == start {
            return vec![start, start];
        }

        if let Some(mut path) = breadth_first_path(successor, start, graph, members) {
            path.insert(0, start);

            return path;
        }
    }

    unreachable!("strongly connected component contains a representative cycle")
}

fn breadth_first_path(
    from: PluginId,
    to: PluginId,
    graph: &Graph,
    members: &BTreeSet<PluginId>,
) -> Option<Vec<PluginId>> {
    let mut queue = VecDeque::from([from]);
    let mut visited = BTreeSet::from([from]);
    let mut parent = BTreeMap::new();

    while let Some(node) = queue.pop_front() {
        for successor in successors(node, graph, members) {
            if successor == to {
                parent.insert(to, node);

                return Some(reconstruct_path(from, to, &parent));
            }

            if visited.insert(successor) {
                parent.insert(successor, node);
                queue.push_back(successor);
            }
        }
    }

    None
}

fn reconstruct_path(
    from: PluginId,
    to: PluginId,
    parent: &BTreeMap<PluginId, PluginId>,
) -> Vec<PluginId> {
    let mut path = vec![to];
    let mut current = to;

    while current != from {
        current = parent[&current];
        path.push(current);
    }

    path.reverse();

    path
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
