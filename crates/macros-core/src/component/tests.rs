use syn::{ImplItem, Item};

use crate::attr::ComponentArgs;
use crate::extend::NoExt;
use crate::paths::Paths;

use super::expand;

#[test]
fn component_impl_items_follow_trait_order() {
    let args = ComponentArgs::<NoExt>::default();
    let item = syn::parse_quote!(
        struct Example;
    );

    let tokens = expand(args, item, &Paths::upwell()).expect("component expands");
    let file = syn::parse2::<syn::File>(tokens).expect("component expansion parses");
    let component_impl = file
        .items
        .iter()
        .find_map(|item| match item {
            Item::Impl(item_impl)
                if item_impl
                    .trait_
                    .as_ref()
                    .and_then(|(_, path, _)| path.segments.last())
                    .is_some_and(|segment| segment.ident == "Component") =>
            {
                Some(item_impl)
            }

            _ => None,
        })
        .expect("Component impl is generated");
    let ordered_items = component_impl
        .items
        .iter()
        .map(|item| match item {
            ImplItem::Type(item) => format!("type {}", item.ident),
            ImplItem::Const(item) => format!("const {}", item.ident),
            ImplItem::Fn(item) => format!("fn {}", item.sig.ident),
            _ => panic!("unexpected Component impl item"),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        ordered_items,
        ["type Handle", "const ID", "const NAME", "fn into_handle"]
    );
}

#[test]
fn linkme_registration_items_allow_their_required_unsafe_attributes() {
    let tokens = expand(
        ComponentArgs::<NoExt>::default(),
        syn::parse_quote!(
            struct Example;
        ),
        &Paths::upwell(),
    )
    .expect("component expands")
    .to_string();

    assert!(
        tokens.contains("allow (unsafe_code)"),
        "component linkme registration scopes its unsafe lint allowance: {tokens}"
    );
}
