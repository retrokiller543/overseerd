//! End-to-end tests for `provide = dyn Trait` injection: primary selection for a
//! single `Arc<dyn Trait>`, and the guarantee that a provider is the *same*
//! instance as the concrete component (an `Arc` alias, never a second build).

use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use overseerd::daemon::App;
use overseerd::{DiError, component, injectable, scope::Transient};

/// A trait two components provide. The `Send + Sync` supertraits make the bare
/// `dyn Animal` shareable, so no use site needs to write `+ Send + Sync`.
#[injectable]
trait Animal: Send + Sync {
    fn sound(&self) -> &'static str;
}

#[injectable]
trait Feline: Animal {}

// `primary` wins a bare `Arc<dyn Animal>`. Qualifier is inferred as the id "dog".
#[component(provide = dyn Animal, primary, after = Cat)]
struct Dog;

impl Animal for Dog {
    fn sound(&self) -> &'static str {
        "woof"
    }
}

// Explicit qualifier overrides the inferred id.
#[component(provide = [dyn Animal, dyn Feline], qualifier = "feline")]
struct Cat;

impl Animal for Cat {
    fn sound(&self) -> &'static str {
        "meow"
    }
}

impl Feline for Cat {}

const EARLY_PRIORITY: i64 = -20;

/// A trait used to prove collection priorities accept const expressions.
#[injectable]
trait PriorityAnimal: Send + Sync {
    fn position(&self) -> &'static str;
}

/// A provider ordered through a module constant expression.
#[component(provide = dyn PriorityAnimal, priority = EARLY_PRIORITY + 5)]
struct EarlyPriorityAnimal;

impl PriorityAnimal for EarlyPriorityAnimal {
    fn position(&self) -> &'static str {
        "early"
    }
}

/// A provider ordered through an associated constant expression.
#[component(
    provide = dyn PriorityAnimal,
    priority = LatePriorityAnimal::PRIORITY
)]
struct LatePriorityAnimal;

impl LatePriorityAnimal {
    const PRIORITY: i64 = 40;
}

impl PriorityAnimal for LatePriorityAnimal {
    fn position(&self) -> &'static str {
        "late"
    }
}

/// Consumes the primary provider, the concrete `Dog`, a `#[qualifier]`-selected
/// provider, and the collection / keyed views of all providers.
#[component]
struct Zoo {
    chosen: Arc<dyn Animal>,
    dog: Arc<Dog>,
    #[qualifier = "feline"]
    cat: Arc<dyn Animal>,
    feline: Arc<dyn Feline>,
    all: Vec<Arc<dyn Animal>>,
    by_name: HashMap<String, Arc<dyn Animal>>,
}

/// The data address an `Arc` points at, erased of pointer metadata, for identity
/// comparison across `Arc<Concrete>` and `Arc<dyn Trait>`.
fn data_ptr<T: ?Sized>(arc: &Arc<T>) -> *const () {
    Arc::as_ptr(arc) as *const ()
}

static TRANSIENT_PROVIDER_IDS: AtomicU64 = AtomicU64::new(1);

/// A unique identity assigned whenever a transient provider is constructed.
struct TransientProviderId(u64);

impl Default for TransientProviderId {
    fn default() -> Self {
        Self(TRANSIENT_PROVIDER_IDS.fetch_add(1, Ordering::Relaxed))
    }
}

/// A trait used to exercise every transient provider projection.
#[injectable]
trait TransientAnimal: Send + Sync {
    fn kind(&self) -> &'static str;
    fn id(&self) -> u64;
}

/// A trait exercising selection-aware build ordering: a single edge constructs
/// only the selected provider, so it must not wait for unselected ones.
#[injectable]
trait CycleProne: Send + Sync {
    fn name(&self) -> &'static str;
}

/// The singleton a single trait edge selects via `primary`.
#[component(provide = dyn CycleProne, primary)]
struct SelectedCycleProvider;

impl CycleProne for SelectedCycleProvider {
    fn name(&self) -> &'static str {
        "selected"
    }
}

/// A singleton depending on the trait through a single edge: it selects the
/// primary provider, never the transient one.
#[component]
struct CycleProneConsumer {
    chosen: Arc<dyn CycleProne>,
}

/// An unselected transient provider depending on the consumer singleton. If the
/// sort waited for every provider's dependencies, the consumer would wait on
/// itself through this provider and the build would report a false cycle.
#[component(
    scope = Transient,
    provide = dyn CycleProne,
    qualifier = "unselected"
)]
struct UnselectedCycleProvider {
    #[expect(unused)]
    consumer: Arc<CycleProneConsumer>,
}

impl CycleProne for UnselectedCycleProvider {
    fn name(&self) -> &'static str {
        "unselected"
    }
}

/// A singleton a transient provider depends on: build-time transient
/// construction must see it in the in-progress scope store, and the topological
/// sort must order consumers after it.
#[component]
struct TransientDependency;

/// A trait whose only provider is a transient with an eager singleton dependency.
#[injectable]
trait DependentAnimal: Send + Sync {
    fn kind(&self) -> &'static str;
}

/// The dependent transient provider.
#[component(
    scope = Transient,
    provide = dyn DependentAnimal,
    qualifier = "dependent"
)]
struct DependentTransientAnimal {
    #[expect(unused)]
    dependency: Arc<TransientDependency>,
}

impl DependentAnimal for DependentTransientAnimal {
    fn kind(&self) -> &'static str {
        "dependent"
    }
}

/// A singleton consuming the dependent transient provider in every shape.
#[component]
struct DependentAnimalViews {
    chosen: Arc<dyn DependentAnimal>,
    #[qualifier = "dependent"]
    qualified: Arc<dyn DependentAnimal>,
    ordered: Vec<Arc<dyn DependentAnimal>>,
    keyed: HashMap<String, Arc<dyn DependentAnimal>>,
}

/// The primary transient provider, ordered after the secondary provider.
#[component(
    scope = Transient,
    provide = dyn TransientAnimal,
    primary,
    after = TransientCat
)]
struct TransientDog {
    #[default]
    id: TransientProviderId,
}

impl TransientAnimal for TransientDog {
    fn kind(&self) -> &'static str {
        "dog"
    }

    fn id(&self) -> u64 {
        self.id.0
    }
}

/// The qualified secondary transient provider.
#[component(
    scope = Transient,
    provide = dyn TransientAnimal,
    qualifier = "transient-cat"
)]
struct TransientCat {
    #[default]
    id: TransientProviderId,
}

impl TransientAnimal for TransientCat {
    fn kind(&self) -> &'static str {
        "cat"
    }

    fn id(&self) -> u64 {
        self.id.0
    }
}

/// A transient consumer covering all supported trait-provider resolution shapes.
#[component(scope = Transient)]
struct TransientProviderViews {
    chosen: Arc<dyn TransientAnimal>,
    #[qualifier = "transient-cat"]
    qualified: Arc<dyn TransientAnimal>,
    ordered: Vec<Arc<dyn TransientAnimal>>,
    keyed: HashMap<String, Arc<dyn TransientAnimal>>,
}

/// A singleton eagerly consuming transient providers: the topological sort must
/// not wedge waiting for transient provider concretes that are never built in a
/// scope, and construction must resolve the providers on demand.
#[component]
struct SingletonTransientViews {
    chosen: Arc<dyn TransientAnimal>,
    #[qualifier = "transient-cat"]
    qualified: Arc<dyn TransientAnimal>,
    ordered: Vec<Arc<dyn TransientAnimal>>,
    keyed: HashMap<String, Arc<dyn TransientAnimal>>,
}

/// A trait whose transient provider always fails construction.
#[injectable]
trait BrokenTransientProvider: Send + Sync {}

/// A transient provider used to verify typed factory-error propagation.
#[component(
    scope = Transient,
    provide = dyn BrokenTransientProvider,
    factory = BrokenTransient::create
)]
struct BrokenTransient;

impl BrokenTransientProvider for BrokenTransient {}

impl BrokenTransient {
    async fn create() -> Result<Self, std::io::Error> {
        Err(std::io::Error::other("transient provider failed"))
    }
}

#[tokio::test]
async fn primary_provider_is_chosen_and_aliases_the_single_instance() {
    let daemon = App::builder("providers-test")
        .auto_discover()
        .build()
        .await
        .expect("daemon builds");

    let zoo = daemon.container().get::<Zoo>().expect("Zoo constructed");

    // `#[primary]` Dog wins the bare `Arc<dyn Animal>` over Cat.
    assert_eq!(zoo.chosen.sound(), "woof", "primary provider chosen");

    // `#[qualifier = "feline"]` selects Cat specifically, ignoring primary.
    assert_eq!(
        zoo.cat.sound(),
        "meow",
        "qualifier selects a specific provider"
    );

    // The trait provider and the concrete dependency are the *same* allocation:
    // the provider is an `Arc` alias of the one constructed Dog, not a rebuild.
    assert_eq!(
        data_ptr(&zoo.chosen),
        data_ptr(&zoo.dog),
        "provider must alias the single concrete instance"
    );

    // The collection follows provider ordering; the keyed view remains indexed by qualifier.
    assert_eq!(zoo.all.len(), 2, "Vec collects all providers");
    let sounds: Vec<&str> = zoo.all.iter().map(|animal| animal.sound()).collect();
    assert_eq!(sounds, ["meow", "woof"]);

    // Inferred id key for Dog; explicit qualifier key for Cat.
    assert_eq!(zoo.by_name["dog"].sound(), "woof");
    assert_eq!(zoo.by_name["feline"].sound(), "meow");

    // The keyed `Dog` entry is still the one shared instance, not a copy.
    assert_eq!(data_ptr(&zoo.by_name["dog"]), data_ptr(&zoo.dog));

    assert_eq!(zoo.feline.sound(), "meow");
    assert_eq!(data_ptr(&zoo.feline), data_ptr(&zoo.cat));
}

#[tokio::test]
async fn provider_priority_accepts_const_and_associated_const_expressions() {
    let daemon = App::builder("provider-priority-test")
        .auto_discover()
        .build()
        .await
        .expect("daemon builds");
    let providers = daemon
        .container()
        .extract::<Vec<Arc<dyn PriorityAnimal>>>()
        .await
        .expect("priority providers resolve");
    let positions: Vec<_> = providers
        .iter()
        .map(|provider| provider.position())
        .collect();

    assert_eq!(positions, ["early", "late"]);
}

#[tokio::test]
async fn transient_providers_are_fresh_erased_and_globally_ordered() {
    let daemon = App::builder("transient-providers-test")
        .auto_discover()
        .build()
        .await
        .expect("daemon builds");

    let first = daemon
        .container()
        .resolve::<Arc<TransientProviderViews>>()
        .await
        .expect("first transient resolution succeeds")
        .expect("first transient consumer resolves");
    let second = daemon
        .container()
        .resolve::<Arc<TransientProviderViews>>()
        .await
        .expect("second transient resolution succeeds")
        .expect("second transient consumer resolves");

    assert_eq!(first.chosen.kind(), "dog");
    assert_eq!(first.qualified.kind(), "cat");
    assert_eq!(
        first
            .ordered
            .iter()
            .map(|provider| provider.kind())
            .collect::<Vec<_>>(),
        ["cat", "dog"]
    );
    assert_eq!(first.keyed["transientdog"].kind(), "dog");
    assert_eq!(first.keyed["transient-cat"].kind(), "cat");

    let first_ids = [
        first.chosen.id(),
        first.qualified.id(),
        first.ordered[0].id(),
        first.ordered[1].id(),
        first.keyed["transientdog"].id(),
        first.keyed["transient-cat"].id(),
    ];
    let mut unique_ids = first_ids.to_vec();

    unique_ids.sort_unstable();
    unique_ids.dedup();

    assert_eq!(unique_ids.len(), first_ids.len());
    assert!(!first_ids.contains(&second.chosen.id()));
    assert!(!first_ids.contains(&second.qualified.id()));
    assert!(
        second
            .ordered
            .iter()
            .all(|provider| !first_ids.contains(&provider.id()))
    );
    assert!(
        second
            .keyed
            .values()
            .all(|provider| !first_ids.contains(&provider.id()))
    );
}

#[tokio::test]
async fn singleton_consumes_transient_providers_eagerly() {
    let daemon = App::builder("singleton-transient-views-test")
        .auto_discover()
        .build()
        .await
        .expect("singleton with eager transient providers builds");

    let views = daemon
        .container()
        .get::<SingletonTransientViews>()
        .expect("singleton transient views constructed");

    assert_eq!(views.chosen.kind(), "dog");
    assert_eq!(views.qualified.kind(), "cat");
    assert_eq!(
        views
            .ordered
            .iter()
            .map(|provider| provider.kind())
            .collect::<Vec<_>>(),
        ["cat", "dog"]
    );
    assert_eq!(views.keyed["transientdog"].kind(), "dog");
    assert_eq!(views.keyed["transient-cat"].kind(), "cat");

    // Every resolution of the collection still constructs fresh transients.
    let first = daemon
        .container()
        .extract::<Vec<Arc<dyn TransientAnimal>>>()
        .await
        .expect("first collection resolves");
    let second = daemon
        .container()
        .extract::<Vec<Arc<dyn TransientAnimal>>>()
        .await
        .expect("second collection resolves");
    let first_ids: Vec<u64> = first.iter().map(|provider| provider.id()).collect();

    assert!(
        second
            .iter()
            .all(|provider| !first_ids.contains(&provider.id()))
    );
}

#[tokio::test]
async fn transient_provider_dependencies_resolve_from_the_building_scope() {
    let daemon = App::builder("dependent-transient-provider-test")
        .auto_discover()
        .build()
        .await
        .expect("singleton consuming a dependent transient provider builds");

    let views = daemon
        .container()
        .get::<DependentAnimalViews>()
        .expect("dependent animal views constructed");

    // Construction succeeding proves the transient provider resolved its own
    // singleton dependency from the in-progress root store (and that the sort
    // ordered the consumer after that dependency).
    assert_eq!(views.chosen.kind(), "dependent");
    assert_eq!(views.qualified.kind(), "dependent");
    assert_eq!(views.ordered.len(), 1);
    assert_eq!(views.keyed["dependent"].kind(), "dependent");
}

#[tokio::test]
async fn single_provider_edges_do_not_wait_for_unselected_transient_providers() {
    let daemon = App::builder("cycle-prone-provider-test")
        .auto_discover()
        .build()
        .await
        .expect("selection-aware ordering reports no false cycle");

    let consumer = daemon
        .container()
        .get::<CycleProneConsumer>()
        .expect("consumer built");

    assert_eq!(consumer.chosen.name(), "selected");
}

#[tokio::test]
async fn transient_provider_factory_errors_remain_typed() {
    let daemon = App::builder("transient-provider-error-test")
        .auto_discover()
        .build()
        .await
        .expect("daemon builds");

    let error = match daemon
        .container()
        .extract::<Arc<dyn BrokenTransientProvider>>()
        .await
    {
        Ok(_) => panic!("broken transient provider must fail"),
        Err(error) => error,
    };

    match error {
        DiError::Other(source) => {
            assert_eq!(source.to_string(), "transient provider failed");
            assert!(source.downcast_ref::<std::io::Error>().is_some());
        }

        other => panic!("expected typed factory error, got {other:?}"),
    }
}
