use super::{Descriptor, RuntimeDescriptor};

/// A type with a statically declared test descriptor.
struct Described;

impl Descriptor<&'static str> for Described {
    const DESCRIPTOR: &'static str = "described";
}

#[test]
fn static_descriptors_are_available_through_trait_objects() {
    let described: &dyn RuntimeDescriptor<&'static str> = &Described;

    assert_eq!(described.descriptor(), "described");
}
