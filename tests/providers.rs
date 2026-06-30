//! End-to-end tests for `provide = dyn Trait` injection: primary selection for a
//! single `Arc<dyn Trait>`, and the guarantee that a provider is the *same*
//! instance as the concrete component (an `Arc` alias, never a second build).

use std::collections::HashMap;
use std::sync::Arc;

use overseerd::daemon::App;
use overseerd::{component, injectable};

/// A trait two components provide. The `Send + Sync` supertraits make the bare
/// `dyn Animal` shareable, so no use site needs to write `+ Send + Sync`.
#[injectable]
trait Animal: Send + Sync {
    fn sound(&self) -> &'static str;
}

#[injectable]
trait Feline: Animal {}

// `primary` wins a bare `Arc<dyn Animal>`. Qualifier is inferred as the id "dog".
#[component(provide = dyn Animal, primary)]
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

    // The collection sees every provider; the keyed view is indexed by qualifier.
    assert_eq!(zoo.all.len(), 2, "Vec collects all providers");

    let mut sounds: Vec<&str> = zoo.all.iter().map(|a| a.sound()).collect();
    sounds.sort_unstable();
    assert_eq!(sounds, ["meow", "woof"]);

    // Inferred id key for Dog; explicit qualifier key for Cat.
    assert_eq!(zoo.by_name["dog"].sound(), "woof");
    assert_eq!(zoo.by_name["feline"].sound(), "meow");

    // The keyed `Dog` entry is still the one shared instance, not a copy.
    assert_eq!(data_ptr(&zoo.by_name["dog"]), data_ptr(&zoo.dog));

    assert_eq!(zoo.feline.sound(), "meow");
    assert_eq!(data_ptr(&zoo.feline), data_ptr(&zoo.cat));
}
