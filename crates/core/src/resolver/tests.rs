use std::sync::Arc;

use super::{Resolver, ResolverSet};

struct First;
impl Resolver for First {}

struct Second;
impl Resolver for Second {}

#[test]
fn clone_shares_the_map_until_a_clone_is_mutated() {
    let mut original = ResolverSet::new();
    original.insert(Arc::new(First));

    let mut clone = original.clone();

    assert!(Arc::ptr_eq(&original.map, &clone.map));

    clone.insert(Arc::new(Second));

    assert!(!Arc::ptr_eq(&original.map, &clone.map));
    assert!(original.get_arc::<First>().is_some());
    assert!(original.get_arc::<Second>().is_none());
    assert!(clone.get_arc::<First>().is_some());
    assert!(clone.get_arc::<Second>().is_some());
}

#[test]
fn resolver_values_are_released_after_the_last_set_clone() {
    let resolver = Arc::new(First);
    let weak = Arc::downgrade(&resolver);
    let mut set = ResolverSet::new();
    set.insert(Arc::clone(&resolver));
    let clone = set.clone();

    drop(resolver);
    drop(set);
    assert!(weak.upgrade().is_some());

    drop(clone);
    assert!(weak.upgrade().is_none());
}
