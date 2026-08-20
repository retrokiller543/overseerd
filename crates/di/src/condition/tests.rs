use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use upwell_core::{
    ConditionDescriptor, ConditionPredicate, ConditionScalar, ConditionScalarKind,
    ConditionScalarLiteral, ConfigFactDescriptor, ConfigFactId, DependencyObservation,
    DescriptorSource, ProviderMappingId, Singleton, Transient, TypeDescriptor,
};

use super::*;
use crate::{
    BoxedComponent, ComponentConstructionContext, ComponentFactoryDescriptor, Dep, FromContainer,
    Live, ProviderOrder,
};

const ENABLED: ConfigFactId = ConfigFactId::new("AuthConfig", "auth", "custom.enabled");
const MODE: ConfigFactId = ConfigFactId::new("AuthConfig", "auth", "mode");
const SOURCE: DescriptorSource = DescriptorSource::new("condition/tests.rs", 1, 1);

fn no_dependencies() -> Vec<upwell_core::DependencyDescriptor> {
    Vec::new()
}

fn missing_dependencies() -> Vec<upwell_core::DependencyDescriptor> {
    vec![crate::dependency_of::<u128>(
        upwell_core::Cardinality::One,
        false,
        false,
    )]
}

fn panic_factory(
    _: &mut ComponentConstructionContext,
) -> Pin<Box<dyn Future<Output = crate::Result<BoxedComponent>> + Send + '_>> {
    panic!("condition evaluation constructed a component")
}

static FACTORY: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
    construct: panic_factory,
    dependencies: no_dependencies,
    default: true,
}];

fn factory() -> &'static [ComponentFactoryDescriptor] {
    &FACTORY
}

static MISSING_FACTORY: [ComponentFactoryDescriptor; 1] = [ComponentFactoryDescriptor {
    construct: panic_factory,
    dependencies: missing_dependencies,
    default: true,
}];

fn missing_factory() -> &'static [ComponentFactoryDescriptor] {
    &MISSING_FACTORY
}

fn erase_panics(_: &BoxedComponent) -> BoxedComponent {
    panic!("condition evaluation erased a provider")
}

fn component<T: 'static>(
    id: &'static str,
    condition: Option<&'static ConditionDescriptor>,
) -> ComponentDescriptor {
    ComponentDescriptor {
        id,
        name: id,
        ty: TypeDescriptor::of::<T>(id),
        scope: &Singleton,
        condition,
        factories: factory,
        hooks: upwell_hooks::no_hooks,
    }
}

fn provider<T: 'static>(
    trait_ty: TypeDescriptor,
    qualifier: &'static str,
    primary: bool,
) -> ProviderDescriptor {
    ProviderDescriptor {
        trait_ty,
        concrete_ty: TypeDescriptor::of::<T>(std::any::type_name::<T>()),
        qualifier,
        primary,
        priority: 0,
        ordering: &[] as &[ProviderOrder],
        erase: erase_panics,
    }
}

fn facts() -> [ConfigFactDescriptor; 2] {
    [
        ConfigFactDescriptor {
            id: ENABLED,
            kind: ConditionScalarKind::Bool,
            source: SOURCE,
        },
        ConfigFactDescriptor {
            id: MODE,
            kind: ConditionScalarKind::String,
            source: SOURCE,
        },
    ]
}

fn snapshot(enabled: bool, mode: &str) -> ConditionFactSnapshot {
    ConditionFactSnapshot::new([
        (ENABLED, ConditionScalar::Bool(enabled)),
        (MODE, ConditionScalar::string(mode)),
    ])
    .expect("facts are unique")
}

struct DefaultAuthenticator;
struct CustomAuthenticator;
trait Authenticator: Send + Sync {}

static CUSTOM_ENABLED: ConditionDescriptor = ConditionDescriptor {
    id: "custom-enabled",
    source: SOURCE,
    predicate: ConditionPredicate::ConfigBool(ENABLED),
};

static DEFAULT_BEFORE_CUSTOM: [ProviderOrder; 1] = [ProviderOrder {
    target: TypeDescriptor::of::<CustomAuthenticator>("CustomAuthenticator"),
    traits: &[TypeDescriptor::of::<dyn Authenticator>("Authenticator")],
    direction: crate::ProviderOrderDirection::Before,
}];

#[test]
fn provider_fallback_uses_existing_selection_after_filtering() {
    let default = component::<DefaultAuthenticator>("default-authenticator", None);
    let custom = component::<CustomAuthenticator>("custom-authenticator", Some(&CUSTOM_ENABLED));
    let providers = vec![
        provider::<DefaultAuthenticator>(
            TypeDescriptor::of::<dyn Authenticator>("Authenticator"),
            "default",
            false,
        ),
        provider::<CustomAuthenticator>(
            TypeDescriptor::of::<dyn Authenticator>("Authenticator"),
            "custom",
            true,
        ),
    ];
    let registry = ComponentRegistry {
        components: vec![custom, default],
        providers,
    };
    let catalog = ConditionCatalog::new(&registry, facts()).expect("catalog validates");

    let disabled = catalog
        .evaluate(&snapshot(false, "safe"))
        .expect("disabled facts evaluate");
    let enabled = catalog
        .evaluate(&snapshot(true, "safe"))
        .expect("enabled facts evaluate");

    assert!(!disabled.component_eligible("custom-authenticator").unwrap());
    assert_eq!(disabled.eligible_registry().providers.len(), 1);
    assert_eq!(
        disabled.eligible_registry().providers[0].qualifier,
        "default"
    );
    assert_eq!(enabled.eligible_registry().providers.len(), 2);

    let enabled_components = enabled
        .eligible_registry()
        .resolved_components()
        .expect("effective components resolve");
    let selection = enabled
        .eligible_registry()
        .provider_selection_model(&enabled_components)
        .expect("existing provider selection accepts effective catalog");

    assert_eq!(
        selection
            .select_runtime_one(
                TypeDescriptor::of::<dyn Authenticator>("Authenticator").type_id,
                None,
                upwell_core::ResolutionMode::Eager,
                &Singleton,
                &|_, _| true,
            )
            .expect("custom is selected")
            .provider
            .qualifier,
        "custom"
    );
}

#[test]
fn ordering_constraints_to_inactive_components_do_not_invalidate_the_candidate() {
    let default = component::<DefaultAuthenticator>("default-authenticator", None);
    let custom = component::<CustomAuthenticator>("custom-authenticator", Some(&CUSTOM_ENABLED));
    let mut default_provider = provider::<DefaultAuthenticator>(
        TypeDescriptor::of::<dyn Authenticator>("Authenticator"),
        "default",
        false,
    );
    default_provider.ordering = &DEFAULT_BEFORE_CUSTOM;
    let registry = ComponentRegistry {
        components: vec![default, custom],
        providers: vec![
            default_provider,
            provider::<CustomAuthenticator>(
                TypeDescriptor::of::<dyn Authenticator>("Authenticator"),
                "custom",
                true,
            ),
        ],
    };
    let catalog = ConditionCatalog::new(&registry, facts()).expect("catalog validates");

    let disabled = catalog
        .evaluate_validated(&snapshot(false, "safe"))
        .expect("inactive ordering target is ignored after complete-catalog validation");
    let enabled = catalog
        .evaluate_validated(&snapshot(true, "safe"))
        .expect("active ordering target retains complete-catalog ordering");

    assert_eq!(disabled.registry().providers.len(), 1);
    assert_eq!(enabled.registry().providers.len(), 2);
}

#[test]
fn inactive_dependencies_are_ignored_but_active_dependencies_still_validate() {
    let mut conditional =
        component::<CustomAuthenticator>("custom-authenticator", Some(&CUSTOM_ENABLED));
    conditional.factories = missing_factory;
    let registry = ComponentRegistry {
        components: vec![conditional],
        providers: Vec::new(),
    };
    let catalog = ConditionCatalog::new(&registry, facts()).expect("catalog validates");

    catalog
        .evaluate_validated(&snapshot(false, "safe"))
        .expect("inactive ordinary dependencies are outside the effective graph");
    assert!(matches!(
        catalog.evaluate_validated(&snapshot(true, "safe")),
        Err(ConditionError::Registry(
            crate::Error::MissingDependency { .. }
        ))
    ));
}

static MODE_EQUALS: ConditionDescriptor = ConditionDescriptor {
    id: "mode-equals",
    source: SOURCE,
    predicate: ConditionPredicate::ConfigEquals {
        fact: MODE,
        expected: ConditionScalarLiteral::String("private-canary"),
    },
};

#[test]
fn fact_values_and_literals_are_redacted() {
    let registry = ComponentRegistry {
        components: vec![component::<CustomAuthenticator>(
            "custom-authenticator",
            Some(&MODE_EQUALS),
        )],
        providers: Vec::new(),
    };
    let catalog = ConditionCatalog::new(&registry, facts()).expect("catalog validates");
    let fact_snapshot = snapshot(false, "runtime-canary");

    let evaluation = catalog.evaluate(&fact_snapshot).expect("facts evaluate");
    let rendered = format!("{fact_snapshot:?} {evaluation:?} {MODE_EQUALS:?}");

    assert!(!rendered.contains("runtime-canary"));
    assert!(!rendered.contains("private-canary"));
}

struct ComponentA;
struct ComponentB;

static A_TO_B: ConditionDescriptor = ConditionDescriptor {
    id: "a-to-b",
    source: SOURCE,
    predicate: ConditionPredicate::ComponentEligible("b"),
};
static B_TO_A: ConditionDescriptor = ConditionDescriptor {
    id: "b-to-a",
    source: SOURCE,
    predicate: ConditionPredicate::Not(&B_TO_A_TARGET),
};
static B_TO_A_TARGET: ConditionDescriptor = ConditionDescriptor {
    id: "b-to-a-target",
    source: SOURCE,
    predicate: ConditionPredicate::ComponentEligible("a"),
};

#[test]
fn availability_cycles_are_rejected_with_canonical_edges() {
    let first = ComponentRegistry {
        components: vec![
            component::<ComponentB>("b", Some(&B_TO_A)),
            component::<ComponentA>("a", Some(&A_TO_B)),
        ],
        providers: Vec::new(),
    };
    let second = ComponentRegistry {
        components: first.components.iter().copied().rev().collect(),
        providers: Vec::new(),
    };

    let first_error = match ConditionCatalog::new(&first, facts()) {
        Ok(_) => panic!("cycle validates"),
        Err(error) => error,
    };
    let second_error = match ConditionCatalog::new(&second, facts()) {
        Ok(_) => panic!("cycle validates"),
        Err(error) => error,
    };

    assert_eq!(first_error.to_string(), second_error.to_string());
    assert!(first_error.to_string().contains("a-to-b"));
    assert!(first_error.to_string().contains("b-to-a-target"));
}

#[test]
fn self_cycles_are_rejected() {
    static SELF: ConditionDescriptor = ConditionDescriptor {
        id: "self",
        source: SOURCE,
        predicate: ConditionPredicate::ComponentEligible("self"),
    };
    let registry = ComponentRegistry {
        components: vec![component::<ComponentA>("self", Some(&SELF))],
        providers: Vec::new(),
    };

    let error = match ConditionCatalog::new(&registry, facts()) {
        Ok(_) => panic!("self-cycle validates"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        ConditionError::AvailabilityCycle(ref edges)
            if edges.len() == 1 && edges[0].from == "self" && edges[0].to == "self"
    ));
}

#[test]
fn manual_and_non_singleton_conditions_are_rejected() {
    let mut manual = ComponentDescriptor::manual(
        "manual",
        "manual",
        TypeDescriptor::of::<ComponentA>("manual"),
        &Singleton,
    );
    manual.condition = Some(&CUSTOM_ENABLED);
    let manual_registry = ComponentRegistry {
        components: vec![manual],
        providers: Vec::new(),
    };
    let mut transient = component::<ComponentB>("transient", Some(&CUSTOM_ENABLED));
    transient.scope = &Transient;
    let transient_registry = ComponentRegistry {
        components: vec![transient],
        providers: Vec::new(),
    };

    assert!(matches!(
        ConditionCatalog::new(&manual_registry, facts()),
        Err(ConditionError::ConditionalManualComponent("manual"))
    ));
    assert!(matches!(
        ConditionCatalog::new(&transient_registry, facts()),
        Err(ConditionError::UnsupportedScope {
            component_id: "transient",
            ..
        })
    ));
}

#[test]
fn manual_registration_cannot_silently_replace_a_conditional_factory() {
    let conditional = component::<ComponentA>("conditional", Some(&CUSTOM_ENABLED));
    let manual = ComponentDescriptor::manual(
        "manual",
        "manual",
        TypeDescriptor::of::<ComponentA>("manual"),
        &Singleton,
    );
    let registry = ComponentRegistry {
        components: vec![conditional, manual],
        providers: Vec::new(),
    };

    assert!(matches!(
        ConditionCatalog::new(&registry, facts()),
        Err(ConditionError::ConditionalManualOverride("conditional"))
    ));
}

#[test]
fn dependency_observation_distinguishes_fixed_and_live_handles() {
    assert_eq!(
        <Arc<ComponentA> as FromContainer>::dependency().observation,
        DependencyObservation::Snapshot
    );
    assert_eq!(
        <Dep<ComponentA> as FromContainer>::dependency().observation,
        DependencyObservation::Live
    );
    assert_eq!(
        <crate::Lazy<Dep<ComponentA>> as FromContainer>::dependency().observation,
        DependencyObservation::Snapshot
    );
    assert_eq!(
        <crate::Deferred<ComponentA> as FromContainer>::dependency().observation,
        DependencyObservation::Snapshot
    );
}

#[test]
fn provider_mapping_identity_is_structural_and_unambiguous() {
    let component = component::<CustomAuthenticator>("custom:auth", None);
    let provider = provider::<CustomAuthenticator>(
        TypeDescriptor::of::<dyn Authenticator>("Authenticator"),
        "qualified:value",
        false,
    );
    let id = provider.mapping_id(&component);

    assert_eq!(id.component, "custom:auth");
    assert_eq!(id.qualifier, "qualified:value");
    assert_eq!(
        id,
        ProviderMappingId::of::<dyn Authenticator>("custom:auth", "qualified:value")
    );
    assert_eq!(
        id.to_string(),
        format!(
            "11#custom:auth{}#{}15#qualified:value",
            id.trait_name().len(),
            id.trait_name()
        )
    );
}

#[test]
fn all_and_any_evaluate_every_child_in_stable_order() {
    static FALSE: ConditionDescriptor = ConditionDescriptor {
        id: "b-false",
        source: SOURCE,
        predicate: ConditionPredicate::ConfigBool(ENABLED),
    };
    static TRUE: ConditionDescriptor = ConditionDescriptor {
        id: "a-true",
        source: SOURCE,
        predicate: ConditionPredicate::ConfigEquals {
            fact: MODE,
            expected: ConditionScalarLiteral::String("safe"),
        },
    };
    static CHILDREN: [ConditionDescriptor; 2] = [FALSE, TRUE];
    static ROOT: ConditionDescriptor = ConditionDescriptor {
        id: "root",
        source: SOURCE,
        predicate: ConditionPredicate::All(&CHILDREN),
    };
    let registry = ComponentRegistry {
        components: vec![component::<ComponentA>("a", Some(&ROOT))],
        providers: Vec::new(),
    };
    let catalog = ConditionCatalog::new(&registry, facts()).expect("catalog validates");

    let evaluation = catalog
        .evaluate(&snapshot(false, "safe"))
        .expect("facts evaluate");
    let ids = evaluation
        .decisions()
        .iter()
        .map(|decision| decision.condition_id)
        .collect::<Vec<_>>();

    assert_eq!(ids, ["a-true", "b-false", "root"]);
    assert_eq!(evaluation.component_eligible("a"), Some(false));
    assert_eq!(
        catalog.dependencies("a").expect("component is known"),
        [
            ConditionDependency::Config(ENABLED),
            ConditionDependency::Config(MODE),
        ]
    );
}

#[test]
fn empty_all_and_any_have_boolean_identity_values() {
    static EMPTY: [ConditionDescriptor; 0] = [];
    static ALL: ConditionDescriptor = ConditionDescriptor {
        id: "empty-all",
        source: SOURCE,
        predicate: ConditionPredicate::All(&EMPTY),
    };
    static ANY: ConditionDescriptor = ConditionDescriptor {
        id: "empty-any",
        source: SOURCE,
        predicate: ConditionPredicate::Any(&EMPTY),
    };
    let all_registry = ComponentRegistry {
        components: vec![component::<ComponentA>("all", Some(&ALL))],
        providers: Vec::new(),
    };
    let any_registry = ComponentRegistry {
        components: vec![component::<ComponentB>("any", Some(&ANY))],
        providers: Vec::new(),
    };

    let all = ConditionCatalog::new(&all_registry, facts())
        .expect("all catalog validates")
        .evaluate(&snapshot(false, "safe"))
        .expect("all evaluates");
    let any = ConditionCatalog::new(&any_registry, facts())
        .expect("any catalog validates")
        .evaluate(&snapshot(false, "safe"))
        .expect("any evaluates");

    assert_eq!(all.component_eligible("all"), Some(true));
    assert_eq!(any.component_eligible("any"), Some(false));
}

#[test]
fn fact_snapshot_rejects_missing_unknown_duplicate_and_wrong_kind_values() {
    let registry = ComponentRegistry {
        components: vec![component::<ComponentA>("a", Some(&CUSTOM_ENABLED))],
        providers: Vec::new(),
    };
    let catalog = ConditionCatalog::new(&registry, facts()).expect("catalog validates");

    assert!(matches!(
        ConditionFactSnapshot::new([
            (ENABLED, ConditionScalar::Bool(true)),
            (ENABLED, ConditionScalar::Bool(false)),
        ]),
        Err(ConditionError::DuplicateFactValue(ENABLED))
    ));
    assert!(matches!(
        catalog.evaluate(
            &ConditionFactSnapshot::new([(ENABLED, ConditionScalar::Bool(true))])
                .expect("one fact is unique")
        ),
        Err(ConditionError::MissingFactValue(MODE))
    ));
    assert!(matches!(
        catalog.evaluate(
            &ConditionFactSnapshot::new([
                (ENABLED, ConditionScalar::string("true")),
                (MODE, ConditionScalar::string("safe")),
            ])
            .expect("facts are unique")
        ),
        Err(ConditionError::FactKindMismatch { fact: ENABLED, .. })
    ));

    const UNKNOWN: ConfigFactId = ConfigFactId::new("Unknown", "unknown", "unknown");
    assert!(matches!(
        catalog.evaluate(
            &ConditionFactSnapshot::new([
                (ENABLED, ConditionScalar::Bool(true)),
                (MODE, ConditionScalar::string("safe")),
                (UNKNOWN, ConditionScalar::Bool(true)),
            ])
            .expect("facts are unique")
        ),
        Err(ConditionError::UnknownFactValue(UNKNOWN))
    ));
}

#[allow(dead_code)]
fn _live_is_not_an_injectable_factory_argument(_: Live<ComponentA>) {}
