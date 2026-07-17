use crate::extend::NoExt;

use super::ComponentArgs;

fn parse_error(input: &str) -> syn::Error {
    match syn::parse_str::<ComponentArgs<NoExt>>(input) {
        Ok(_) => panic!("component arguments unexpectedly parsed"),
        Err(error) => error,
    }
}

#[test]
fn by_value_component_cannot_register_trait_providers() {
    let error = parse_error("by_value, provide = dyn Repository");

    assert!(
        error
            .to_string()
            .contains("`by_value` cannot be combined with `provide`")
    );
}

#[test]
fn validation_is_independent_of_argument_order() {
    let error = parse_error("provide = [dyn Repository, dyn Cache], by_value");

    assert!(
        error
            .to_string()
            .contains("`by_value` cannot be combined with `provide`")
    );
}
