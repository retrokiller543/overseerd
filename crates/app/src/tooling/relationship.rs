use std::collections::{BTreeMap, BTreeSet};

use upwell_tooling_schema::{Relationship, RelationshipKind};

use super::Projection;
use crate::ProtocolDefinition;

impl<D: ProtocolDefinition> Projection<'_, D> {
    pub(super) fn relationship<const N: usize>(
        &mut self,
        kind: RelationshipKind,
        from: &str,
        to: &str,
        labels: [(String, String); N],
    ) {
        self.relationship_map(kind, from, to, BTreeMap::from(labels));
    }

    pub(super) fn relationship_map(
        &mut self,
        kind: RelationshipKind,
        from: &str,
        to: &str,
        labels: BTreeMap<String, String>,
    ) {
        if let Some(relationship) = self.document.relationships.iter_mut().find(|relationship| {
            relationship.kind == kind && relationship.from == from && relationship.to == to
        }) {
            merge_relationship_labels(&kind, &mut relationship.labels, labels);

            return;
        }

        self.document.relationships.push(Relationship {
            kind,
            from: from.to_string(),
            to: to.to_string(),
            labels,
        });
    }
}

fn merge_relationship_labels(
    kind: &RelationshipKind,
    existing: &mut BTreeMap<String, String>,
    incoming: BTreeMap<String, String>,
) {
    if *kind == RelationshipKind::DependsOn {
        merge_dependency_labels(existing, incoming);

        return;
    }

    merge_scalar_labels(existing, incoming);
}

fn merge_dependency_labels(
    existing: &mut BTreeMap<String, String>,
    incoming: BTreeMap<String, String>,
) {
    const DECISIONS: &str = "dependency-decisions";

    let existing_decisions = existing.remove(DECISIONS);
    let mut decisions = match existing_decisions.as_deref() {
        Some(value) => serde_json::from_str::<Vec<BTreeMap<String, String>>>(value)
            .expect("projection-authored dependency decisions remain valid JSON"),
        None => vec![existing.clone()],
    };

    if decisions.contains(&incoming) {
        if let Some(value) = existing_decisions {
            existing.insert(String::from(DECISIONS), value);
        }

        return;
    }

    decisions.push(incoming);
    decisions.sort();
    decisions.dedup();

    let shared = decisions.first().map_or_else(BTreeMap::new, |first| {
        first
            .iter()
            .filter(|(name, value)| {
                decisions
                    .iter()
                    .all(|decision| decision.get(*name) == Some(*value))
            })
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    });

    *existing = shared;
    existing.insert(
        String::from(DECISIONS),
        serde_json::to_string(&decisions).expect("dependency decision label maps always serialize"),
    );
}

fn merge_scalar_labels(
    existing: &mut BTreeMap<String, String>,
    incoming: BTreeMap<String, String>,
) {
    let names = existing
        .keys()
        .chain(incoming.keys())
        .map(|name| {
            name.strip_suffix("-values")
                .unwrap_or(name.as_str())
                .to_string()
        })
        .collect::<BTreeSet<_>>();

    for name in names {
        let aggregate = format!("{name}-values");

        if let Some(aggregate_value) = existing.get(&aggregate) {
            let mut values: Vec<Option<String>> = serde_json::from_str(aggregate_value)
                .expect("projection-authored aggregate labels remain valid JSON");

            values.push(incoming.get(&name).cloned());
            values.sort();
            values.dedup();
            existing.remove(&name);
            existing.insert(
                aggregate,
                serde_json::to_string(&values).expect("string label values always serialize"),
            );

            continue;
        }

        let left = existing.get(&name).cloned();
        let right = incoming.get(&name).cloned();

        if left == right {
            continue;
        }

        let mut values = vec![left, right];

        values.sort();
        values.dedup();
        existing.remove(&name);
        existing.insert(
            aggregate,
            serde_json::to_string(&values).expect("string label values always serialize"),
        );
    }
}

#[cfg(test)]
mod tests;
