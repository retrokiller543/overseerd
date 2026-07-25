use super::*;

const PROTOCOL: ProtocolId = match ProtocolId::new("test/protocol") {
    Ok(id) => id,
    Err(_) => panic!("valid protocol id"),
};

fn plugin_id(value: &'static str) -> PluginId {
    PluginId::new(value).expect("valid test plugin id")
}

fn slot_id(value: &'static str) -> PluginSlotId {
    PluginSlotId::new(value).expect("valid test slot id")
}

fn contribution_id(value: &'static str) -> ContributionId {
    ContributionId::new(value).expect("valid test contribution id")
}

fn early(ordinal: u32) -> InstallationProvenance {
    InstallationProvenance::new(InstallationOrigin::ApplicationDeclaration, ordinal)
}

fn late(ordinal: u32) -> InstallationProvenance {
    InstallationProvenance::new(InstallationOrigin::ApplicationConfiguration, ordinal)
}

fn install(id: &'static str, provenance: InstallationProvenance) -> CompositionDirective {
    CompositionDirective::install(PluginDeclaration::new(plugin_id(id), provenance))
}

fn install_with(
    id: &'static str,
    provenance: InstallationProvenance,
    relations: impl IntoIterator<Item = PluginRelation>,
) -> CompositionDirective {
    CompositionDirective::install(
        PluginDeclaration::new(plugin_id(id), provenance).with_relations(relations),
    )
}

fn relation(kind: RelationKind, target: RelationTarget) -> PluginRelation {
    PluginRelation::new(kind, target)
}

fn plugin_target(id: &'static str) -> RelationTarget {
    RelationTarget::Plugin(plugin_id(id))
}

fn slot_target(id: &'static str) -> RelationTarget {
    RelationTarget::Slot(slot_id(id))
}

fn ids(plan: &[ResolvedPlugin]) -> Vec<PluginId> {
    plan.iter().map(ResolvedPlugin::id).collect()
}

fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    fn visit<T: Clone>(start: usize, items: &mut Vec<T>, output: &mut Vec<Vec<T>>) {
        if start == items.len() {
            output.push(items.clone());

            return;
        }

        for index in start..items.len() {
            items.swap(start, index);
            visit(start + 1, items, output);
            items.swap(start, index);
        }
    }

    let mut items = items.to_vec();
    let mut output = Vec::new();

    visit(0, &mut items, &mut output);

    output
}

#[test]
fn stable_ids_validate_namespaced_ascii_paths() {
    let valid = PluginId::new("acme/rpc.router-v2").expect("valid id");
    let missing = PluginId::new("router").expect_err("namespace is required");
    let uppercase = PluginId::new("Acme/router").expect_err("uppercase is rejected");
    let empty = PluginId::new("acme//router").expect_err("empty segment is rejected");
    let unicode = PluginId::new("acme/routér").expect_err("unicode is rejected");

    assert_eq!(valid.as_str(), "acme/rpc.router-v2");
    assert_eq!(valid.to_string(), "acme/rpc.router-v2");
    assert_eq!(missing.kind(), IdErrorKind::MissingNamespace);
    assert_eq!(uppercase.kind(), IdErrorKind::InvalidSegmentStart);
    assert_eq!(empty.kind(), IdErrorKind::MissingNamespace);
    assert_eq!(unicode.kind(), IdErrorKind::InvalidCharacter);
}

#[test]
fn application_declarations_cannot_claim_framework_namespace() {
    let diagnostics = resolve_early_plugins(PROTOCOL, [install("overseerd/router", early(0))])
        .expect_err("framework namespace is reserved");
    let framework = InstallationProvenance::new(InstallationOrigin::Framework, 0);
    let plan = resolve_early_plugins(PROTOCOL, [install("overseerd/router", framework)])
        .expect("framework may use reserved namespace");

    assert!(matches!(
        diagnostics.as_slice(),
        [CompositionDiagnostic::ReservedNamespace { .. }]
    ));
    assert_eq!(plan.plugins()[0].id(), plugin_id("overseerd/router"));
}

#[test]
fn identifiers_are_category_safe_and_lexically_ordered() {
    let alpha = plugin_id("test/alpha");
    let beta = plugin_id("test/beta");
    let protocol = PROTOCOL;

    assert!(alpha < beta);
    assert_eq!(protocol.as_str(), "test/protocol");
    assert_eq!(slot_id("test/alpha").as_str(), alpha.as_str());
}

#[test]
fn contribution_identity_is_local_to_explicit_contributor() {
    let id = contribution_id("test/routes");
    let app = ContributionProvenance::new(Contributor::Application, id);
    let plugin = ContributionProvenance::new(Contributor::Plugin(plugin_id("test/router")), id);

    assert_ne!(app, plugin);
    assert_eq!(app.contribution(), plugin.contribution());
    assert_eq!(
        plugin.contributor(),
        Contributor::Plugin(plugin_id("test/router"))
    );
}

#[test]
fn relation_order_and_duplicates_do_not_change_declaration_identity() {
    let before = relation(RelationKind::Before, plugin_target("test/beta"));
    let requires = relation(RelationKind::Requires, slot_target("test/router"));
    let left = PluginDeclaration::new(plugin_id("test/alpha"), early(0))
        .with_relations([requires, before, requires]);
    let right = PluginDeclaration::new(plugin_id("test/alpha"), early(0))
        .relates(before)
        .relates(requires);

    assert_eq!(left, right);
    assert_eq!(left.relations(), &[requires, before]);
}

#[test]
fn independent_plugins_use_lexical_identity_order() {
    let plan = resolve_early_plugins(
        PROTOCOL,
        [
            install("test/zulu", early(0)),
            install("test/alpha", early(2)),
            install("test/middle", early(1)),
        ],
    )
    .expect("composition resolves");

    assert_eq!(
        ids(plan.plugins()),
        [
            plugin_id("test/alpha"),
            plugin_id("test/middle"),
            plugin_id("test/zulu"),
        ]
    );
}

#[test]
fn dependency_and_ordering_edges_form_one_deterministic_graph() {
    let plan = resolve_early_plugins(
        PROTOCOL,
        [
            install_with(
                "test/gamma",
                early(2),
                [relation(RelationKind::After, plugin_target("test/alpha"))],
            ),
            install_with(
                "test/beta",
                early(1),
                [relation(
                    RelationKind::Requires,
                    plugin_target("test/gamma"),
                )],
            ),
            install("test/alpha", early(0)),
        ],
    )
    .expect("composition resolves");

    assert_eq!(
        ids(plan.plugins()),
        [
            plugin_id("test/alpha"),
            plugin_id("test/gamma"),
            plugin_id("test/beta"),
        ]
    );
}

#[test]
fn absent_order_and_conflict_targets_are_optional_but_dependencies_are_not() {
    let valid = resolve_early_plugins(
        PROTOCOL,
        [install_with(
            "test/source",
            early(0),
            [
                relation(RelationKind::Before, plugin_target("test/absent")),
                relation(RelationKind::Conflicts, slot_target("test/optional")),
            ],
        )],
    )
    .expect("optional relations ignore missing targets");
    let invalid = resolve_early_plugins(
        PROTOCOL,
        [install_with(
            "test/source",
            early(0),
            [relation(
                RelationKind::Requires,
                plugin_target("test/absent"),
            )],
        )],
    )
    .expect_err("hard dependency requires its target");

    assert_eq!(valid.plugins().len(), 1);
    assert!(matches!(
        invalid.as_slice(),
        [CompositionDiagnostic::MissingDependency { .. }]
    ));
}

#[test]
fn replacement_preserves_implementation_and_slot_identity() {
    let router_slot = slot_id("test/router");
    let default = PluginDeclaration::new(plugin_id("test/default-router"), early(0))
        .provides(router_slot, SlotPolicy::Replaceable);
    let replacement = PluginDeclaration::new(plugin_id("acme/radix-router"), early(1));
    let consumer = PluginDeclaration::new(plugin_id("test/consumer"), early(2)).relates(relation(
        RelationKind::Requires,
        RelationTarget::Slot(router_slot),
    ));
    let plan = resolve_early_plugins(
        PROTOCOL,
        [
            CompositionDirective::install(default),
            CompositionDirective::replace(router_slot, replacement),
            CompositionDirective::install(consumer),
        ],
    )
    .expect("replacement resolves");
    let decision = plan.replacements()[0];

    assert!(plan.plugin(plugin_id("test/default-router")).is_none());
    assert_eq!(
        plan.slot(router_slot).map(ResolvedPlugin::id),
        Some(plugin_id("acme/radix-router"))
    );
    assert_eq!(decision.replaced(), plugin_id("test/default-router"));
    assert_eq!(decision.replaced_provenance(), early(0));
    assert_eq!(decision.replacement(), plugin_id("acme/radix-router"));
    assert_eq!(
        ids(plan.plugins()),
        [plugin_id("acme/radix-router"), plugin_id("test/consumer")]
    );
}

#[test]
fn replacement_may_declare_the_slot_it_provides() {
    let router_slot = slot_id("test/router");
    let plan = resolve_early_plugins(
        PROTOCOL,
        [
            CompositionDirective::install(
                PluginDeclaration::new(plugin_id("test/default-router"), early(0))
                    .provides(router_slot, SlotPolicy::Replaceable),
            ),
            CompositionDirective::replace(
                router_slot,
                PluginDeclaration::new(plugin_id("acme/router"), early(1))
                    .provides(router_slot, SlotPolicy::Replaceable),
            ),
        ],
    )
    .expect("matching replacement slot resolves");

    assert_eq!(
        plan.slot(router_slot).map(ResolvedPlugin::id),
        Some(plugin_id("acme/router"))
    );
}

#[test]
fn optional_slots_can_be_suppressed_but_fixed_slots_cannot_change() {
    let optional_slot = slot_id("test/openapi");
    let fixed_slot = slot_id("test/runtime");
    let optional = PluginDeclaration::new(plugin_id("test/openapi"), early(0))
        .provides(optional_slot, SlotPolicy::Optional);
    let fixed = PluginDeclaration::new(plugin_id("test/runtime"), early(1))
        .provides(fixed_slot, SlotPolicy::Fixed);
    let valid = resolve_early_plugins(
        PROTOCOL,
        [
            CompositionDirective::install(optional.clone()),
            CompositionDirective::suppress(optional_slot, early(2)),
        ],
    )
    .expect("optional slot suppresses");
    let invalid_replacement = resolve_early_plugins(
        PROTOCOL,
        [
            CompositionDirective::install(fixed.clone()),
            CompositionDirective::replace(
                fixed_slot,
                PluginDeclaration::new(plugin_id("test/other-runtime"), early(2)),
            ),
        ],
    )
    .expect_err("fixed slot rejects replacement");
    let invalid_suppression = resolve_early_plugins(
        PROTOCOL,
        [
            CompositionDirective::install(fixed),
            CompositionDirective::install(optional),
            CompositionDirective::suppress(fixed_slot, early(3)),
        ],
    )
    .expect_err("fixed slot rejects suppression");

    assert!(valid.slot(optional_slot).is_none());
    assert_eq!(
        valid.suppressions()[0].suppressed(),
        plugin_id("test/openapi")
    );
    assert_eq!(valid.suppressions()[0].suppressed_provenance(), early(0));
    assert!(
        invalid_replacement
            .as_slice()
            .iter()
            .any(|item| matches!(item, CompositionDiagnostic::ReplacementForbidden { .. }))
    );
    assert!(
        invalid_suppression
            .as_slice()
            .iter()
            .any(|item| matches!(item, CompositionDiagnostic::SuppressionForbidden { .. }))
    );
}

#[test]
fn duplicate_and_conflict_diagnostics_name_both_origins() {
    let duplicate = plugin_id("test/duplicate");
    let conflict_a = plugin_id("test/conflict-a");
    let conflict_b = plugin_id("test/conflict-b");
    let diagnostics = resolve_early_plugins(
        PROTOCOL,
        [
            install(duplicate.as_str(), early(9)),
            install(duplicate.as_str(), early(1)),
        ],
    )
    .expect_err("duplicate fails");
    let conflicts = resolve_early_plugins(
        PROTOCOL,
        [
            install_with(
                conflict_b.as_str(),
                early(1),
                [relation(
                    RelationKind::Conflicts,
                    RelationTarget::Plugin(conflict_a),
                )],
            ),
            install(conflict_a.as_str(), early(0)),
        ],
    )
    .expect_err("conflict fails");

    assert!(matches!(
        diagnostics.as_slice(),
        [CompositionDiagnostic::DuplicatePlugin { first, second, .. }]
            if first.ordinal() == 1 && second.ordinal() == 9
    ));
    assert!(matches!(
        conflicts.as_slice(),
        [CompositionDiagnostic::Conflict { left, right, .. }]
            if *left == conflict_a && *right == conflict_b
    ));
}

#[test]
fn cycles_report_only_the_cyclic_component_with_typed_edges() {
    let directives = [
        install_with(
            "test/alpha",
            early(0),
            [
                relation(RelationKind::Requires, plugin_target("test/beta")),
                relation(RelationKind::Before, plugin_target("test/beta")),
            ],
        ),
        install("test/beta", early(1)),
        install_with(
            "test/downstream",
            early(2),
            [relation(
                RelationKind::Requires,
                plugin_target("test/alpha"),
            )],
        ),
    ];
    let diagnostics = resolve_early_plugins(PROTOCOL, directives).expect_err("cycle fails");
    let CompositionDiagnostic::Cycle {
        members,
        steps,
        display,
        ..
    } = &diagnostics.as_slice()[0]
    else {
        panic!("expected cycle diagnostic");
    };

    assert_eq!(display, "test/alpha -> test/beta -> test/alpha");
    assert_eq!(members.len(), 2);
    assert_eq!(members[0].plugin(), plugin_id("test/alpha"));
    assert_eq!(members[0].provenance(), early(0));
    assert_eq!(steps.len(), 2);
    assert!(
        steps
            .iter()
            .all(|step| step.from().plugin() != plugin_id("test/downstream"))
    );
    assert!(
        steps
            .iter()
            .any(|step| step.kinds().contains(&RelationKind::Requires))
    );
    assert!(
        steps
            .iter()
            .any(|step| step.kinds().contains(&RelationKind::Before))
    );
}

#[test]
fn cycle_members_include_every_plugin_in_the_cyclic_component() {
    let diagnostics = resolve_early_plugins(
        PROTOCOL,
        [
            install_with(
                "test/alpha",
                early(0),
                [
                    relation(RelationKind::Before, plugin_target("test/beta")),
                    relation(RelationKind::After, plugin_target("test/beta")),
                ],
            ),
            install_with(
                "test/beta",
                early(1),
                [
                    relation(RelationKind::Before, plugin_target("test/gamma")),
                    relation(RelationKind::After, plugin_target("test/gamma")),
                ],
            ),
            install("test/gamma", early(2)),
        ],
    )
    .expect_err("strongly connected graph fails");
    let CompositionDiagnostic::Cycle { members, .. } = &diagnostics.as_slice()[0] else {
        panic!("expected cycle diagnostic");
    };

    assert_eq!(
        members
            .iter()
            .map(|member| member.plugin())
            .collect::<Vec<_>>(),
        [
            plugin_id("test/alpha"),
            plugin_id("test/beta"),
            plugin_id("test/gamma"),
        ]
    );
}

#[test]
fn late_composition_extends_early_order_monotonically() {
    let early_plan = resolve_early_plugins(
        PROTOCOL,
        [
            install("test/zulu-early", early(0)),
            install("test/alpha-early", early(1)),
        ],
    )
    .expect("early plan resolves");
    let final_plan = extend_late_plugins(
        &early_plan,
        [
            install_with(
                "test/zulu-late",
                late(0),
                [relation(
                    RelationKind::Requires,
                    plugin_target("test/alpha-early"),
                )],
            ),
            install("test/alpha-late", late(1)),
        ],
    )
    .expect("late plan resolves");

    assert_eq!(
        ids(early_plan.plugins()),
        [plugin_id("test/alpha-early"), plugin_id("test/zulu-early")]
    );
    assert_eq!(
        ids(final_plan.plugins()),
        [
            plugin_id("test/alpha-early"),
            plugin_id("test/zulu-early"),
            plugin_id("test/alpha-late"),
            plugin_id("test/zulu-late"),
        ]
    );
}

#[test]
fn late_plugins_cannot_mutate_or_reorder_early_capabilities() {
    let slot = slot_id("test/router");
    let early_plan = resolve_early_plugins(
        PROTOCOL,
        [CompositionDirective::install(
            PluginDeclaration::new(plugin_id("test/router"), early(0))
                .provides(slot, SlotPolicy::Optional),
        )],
    )
    .expect("early plan resolves");
    let diagnostics = extend_late_plugins(
        &early_plan,
        [
            CompositionDirective::replace(
                slot,
                PluginDeclaration::new(plugin_id("test/late-router"), late(0)),
            ),
            install_with(
                "test/late-before",
                late(1),
                [relation(RelationKind::Before, plugin_target("test/router"))],
            ),
        ],
    )
    .expect_err("late mutations fail");

    assert_eq!(diagnostics.len(), 2);
    assert!(
        diagnostics
            .as_slice()
            .iter()
            .any(|item| matches!(item, CompositionDiagnostic::LateSlotMutation { .. }))
    );
    assert!(
        diagnostics
            .as_slice()
            .iter()
            .any(|item| matches!(item, CompositionDiagnostic::LateOrderingBeforeEarly { .. }))
    );
}

#[test]
fn late_plugins_cannot_reintroduce_an_early_suppressed_slot() {
    let slot = slot_id("test/openapi");
    let early_plan = resolve_early_plugins(
        PROTOCOL,
        [
            CompositionDirective::install(
                PluginDeclaration::new(plugin_id("test/openapi"), early(0))
                    .provides(slot, SlotPolicy::Optional),
            ),
            CompositionDirective::suppress(slot, early(1)),
        ],
    )
    .expect("early slot suppresses");
    let diagnostics = extend_late_plugins(
        &early_plan,
        [CompositionDirective::install(
            PluginDeclaration::new(plugin_id("test/late-openapi"), late(0))
                .provides(slot, SlotPolicy::Optional),
        )],
    )
    .expect_err("late declaration cannot restore suppressed early slot");

    assert!(matches!(
        diagnostics.as_slice(),
        [CompositionDiagnostic::LateSlotMutation { .. }]
    ));
}

#[test]
fn structural_errors_do_not_emit_cascade_cycle_diagnostics() {
    let diagnostics = resolve_early_plugins(
        PROTOCOL,
        [
            install_with(
                "test/alpha",
                early(0),
                [relation(RelationKind::Before, plugin_target("test/beta"))],
            ),
            install_with(
                "test/beta",
                early(1),
                [relation(RelationKind::Before, plugin_target("test/alpha"))],
            ),
            install("test/alpha", early(2)),
        ],
    )
    .expect_err("duplicate identity makes graph ambiguous");

    assert!(
        diagnostics
            .as_slice()
            .iter()
            .any(|item| matches!(item, CompositionDiagnostic::DuplicatePlugin { .. }))
    );
    assert!(
        !diagnostics
            .as_slice()
            .iter()
            .any(|item| matches!(item, CompositionDiagnostic::Cycle { .. }))
    );
}

#[test]
fn competing_slot_directives_do_not_emit_cascade_cycles() {
    let slot = slot_id("test/router");
    let diagnostics = resolve_early_plugins(
        PROTOCOL,
        [
            CompositionDirective::install(
                PluginDeclaration::new(plugin_id("test/default-router"), early(0))
                    .provides(slot, SlotPolicy::Replaceable)
                    .relates(relation(
                        RelationKind::Before,
                        plugin_target("test/consumer"),
                    )),
            ),
            CompositionDirective::replace(
                slot,
                PluginDeclaration::new(plugin_id("test/router-a"), early(1)),
            ),
            CompositionDirective::replace(
                slot,
                PluginDeclaration::new(plugin_id("test/router-b"), early(2)),
            ),
            install_with(
                "test/consumer",
                early(3),
                [relation(RelationKind::Before, slot_target("test/router"))],
            ),
        ],
    )
    .expect_err("competing replacements fail");

    assert!(
        diagnostics
            .as_slice()
            .iter()
            .any(|item| matches!(item, CompositionDiagnostic::MultipleReplacements { .. }))
    );
    assert!(
        !diagnostics
            .as_slice()
            .iter()
            .any(|item| matches!(item, CompositionDiagnostic::Cycle { .. }))
    );
}

#[test]
fn duplicate_suppressions_do_not_hide_independent_cycles() {
    let slot = slot_id("test/openapi");
    let diagnostics = resolve_early_plugins(
        PROTOCOL,
        [
            CompositionDirective::install(
                PluginDeclaration::new(plugin_id("test/openapi"), early(0))
                    .provides(slot, SlotPolicy::Optional),
            ),
            CompositionDirective::suppress(slot, early(1)),
            CompositionDirective::suppress(slot, early(2)),
            install_with(
                "test/alpha",
                early(3),
                [relation(RelationKind::Before, plugin_target("test/beta"))],
            ),
            install_with(
                "test/beta",
                early(4),
                [relation(RelationKind::Before, plugin_target("test/alpha"))],
            ),
        ],
    )
    .expect_err("independent failures are aggregated");

    assert!(
        diagnostics
            .as_slice()
            .iter()
            .any(|item| matches!(item, CompositionDiagnostic::MultipleSuppressions { .. }))
    );
    assert!(
        diagnostics
            .as_slice()
            .iter()
            .any(|item| matches!(item, CompositionDiagnostic::Cycle { .. }))
    );
}

#[test]
fn unrelated_validation_errors_do_not_hide_independent_cycles() {
    let diagnostics = resolve_early_plugins(
        PROTOCOL,
        [
            install_with(
                "test/consumer",
                early(0),
                [relation(
                    RelationKind::Requires,
                    plugin_target("test/missing"),
                )],
            ),
            install_with(
                "test/alpha",
                early(1),
                [relation(RelationKind::Before, plugin_target("test/beta"))],
            ),
            install_with(
                "test/beta",
                early(2),
                [relation(RelationKind::Before, plugin_target("test/alpha"))],
            ),
        ],
    )
    .expect_err("independent failures are aggregated");

    assert!(
        diagnostics
            .as_slice()
            .iter()
            .any(|item| matches!(item, CompositionDiagnostic::MissingDependency { .. }))
    );
    assert!(
        diagnostics
            .as_slice()
            .iter()
            .any(|item| matches!(item, CompositionDiagnostic::Cycle { .. }))
    );
}

#[test]
fn deep_cycles_return_typed_diagnostics_without_recursive_traversal() {
    const NODE_COUNT: usize = 4_096;

    let ids: Vec<_> = (0..NODE_COUNT)
        .map(|index| {
            let text: &'static str = Box::leak(format!("deep/plugin-{index:04}").into_boxed_str());

            plugin_id(text)
        })
        .collect();
    let directives: Vec<_> = ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            let target = ids[(index + 1) % NODE_COUNT];

            CompositionDirective::install(PluginDeclaration::new(*id, early(index as u32)).relates(
                relation(RelationKind::Before, RelationTarget::Plugin(target)),
            ))
        })
        .collect();
    let diagnostics = resolve_early_plugins(PROTOCOL, directives).expect_err("cycle fails");

    assert!(matches!(
        diagnostics.as_slice(),
        [CompositionDiagnostic::Cycle { members, .. }] if members.len() == NODE_COUNT
    ));
}

#[test]
fn early_conflicts_are_rechecked_against_late_installations() {
    let early_plan = resolve_early_plugins(
        PROTOCOL,
        [install_with(
            "test/early",
            early(0),
            [relation(
                RelationKind::Conflicts,
                plugin_target("test/late"),
            )],
        )],
    )
    .expect("absent early conflict target is allowed");
    let diagnostics = extend_late_plugins(&early_plan, [install("test/late", late(0))])
        .expect_err("late target activates early conflict");

    assert!(matches!(
        diagnostics.as_slice(),
        [CompositionDiagnostic::Conflict { .. }]
    ));
}

#[test]
fn early_after_relation_rejects_a_new_late_target() {
    let early_plan = resolve_early_plugins(
        PROTOCOL,
        [install_with(
            "test/early",
            early(0),
            [relation(RelationKind::After, plugin_target("test/late"))],
        )],
    )
    .expect("absent ordering target is allowed");
    let diagnostics = extend_late_plugins(&early_plan, [install("test/late", late(0))])
        .expect_err("late target cannot reorder early plan");

    assert!(matches!(
        diagnostics.as_slice(),
        [CompositionDiagnostic::EarlyOrderingAfterLate { .. }]
    ));
}

#[test]
fn aggregate_report_includes_independent_duplicate_and_dependency_failures() {
    let diagnostics = resolve_early_plugins(
        PROTOCOL,
        [
            install("test/duplicate", early(0)),
            install("test/duplicate", early(1)),
            install_with(
                "test/consumer",
                early(2),
                [relation(
                    RelationKind::Requires,
                    plugin_target("test/missing"),
                )],
            ),
        ],
    )
    .expect_err("invalid graph fails");

    assert!(
        diagnostics
            .as_slice()
            .iter()
            .any(|item| matches!(item, CompositionDiagnostic::DuplicatePlugin { .. }))
    );
    assert!(
        diagnostics
            .as_slice()
            .iter()
            .any(|item| matches!(item, CompositionDiagnostic::MissingDependency { .. }))
    );
}

#[test]
fn plans_and_diagnostics_are_invariant_under_all_input_permutations() {
    let router_slot = slot_id("test/router");
    let valid = vec![
        CompositionDirective::install(
            PluginDeclaration::new(plugin_id("test/default-router"), early(0))
                .provides(router_slot, SlotPolicy::Replaceable),
        ),
        CompositionDirective::replace(
            router_slot,
            PluginDeclaration::new(plugin_id("test/replacement"), early(1)),
        ),
        install_with(
            "test/consumer",
            early(2),
            [relation(
                RelationKind::Requires,
                RelationTarget::Slot(router_slot),
            )],
        ),
        install("test/independent", early(3)),
    ];
    let baseline = resolve_early_plugins(PROTOCOL, valid.clone()).expect("baseline resolves");

    for permutation in permutations(&valid) {
        assert_eq!(
            resolve_early_plugins(PROTOCOL, permutation).expect("permutation resolves"),
            baseline
        );
    }

    let invalid = vec![
        install_with(
            "test/alpha",
            early(0),
            [relation(
                RelationKind::Requires,
                plugin_target("test/missing"),
            )],
        ),
        install("test/duplicate", early(1)),
        install("test/duplicate", early(2)),
    ];
    let baseline = resolve_early_plugins(PROTOCOL, invalid.clone()).expect_err("baseline fails");

    for permutation in permutations(&invalid) {
        let diagnostics =
            resolve_early_plugins(PROTOCOL, permutation).expect_err("permutation fails");

        assert_eq!(diagnostics, baseline);
        assert_eq!(diagnostics.to_string(), baseline.to_string());
    }
}

#[test]
fn backend_shaped_enumeration_orders_produce_identical_plans() {
    let linkme = vec![
        install("test/alpha", early(10)),
        install_with(
            "test/gamma",
            early(30),
            [relation(RelationKind::After, plugin_target("test/beta"))],
        ),
        install("test/beta", early(20)),
    ];
    let inventory = linkme.iter().cloned().rev().collect::<Vec<_>>();

    assert_eq!(
        resolve_early_plugins(PROTOCOL, linkme).expect("linkme-shaped plan"),
        resolve_early_plugins(PROTOCOL, inventory).expect("inventory-shaped plan")
    );
}
