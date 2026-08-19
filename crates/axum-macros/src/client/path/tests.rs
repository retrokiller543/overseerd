use quote::quote;

use super::{hole_ident, parse_template, plan_path, tuple_elems};

fn plan_args(holes: &[&str], path_ty: Option<syn::Type>) -> Vec<String> {
    let holes = holes
        .iter()
        .map(|hole| hole.to_string())
        .collect::<Vec<_>>();

    plan_path(&holes, path_ty)
        .args
        .iter()
        .map(|(name, ty)| format!("{}: {}", name, quote!(#ty)))
        .collect()
}

#[test]
fn single_hole_uses_path_type() {
    assert_eq!(
        plan_args(&["id"], Some(syn::parse_quote!(u64))),
        ["id: u64"]
    );
}

#[test]
fn guarded_hole_falls_back_to_string_param() {
    assert_eq!(plan_args(&["id"], None), ["id: :: std :: string :: String"]);
}

#[test]
fn multiple_guarded_holes_fall_back_to_string_params() {
    assert_eq!(
        plan_args(&["id", "child"], None),
        [
            "id: :: std :: string :: String",
            "child: :: std :: string :: String"
        ]
    );
}

#[test]
fn matching_tuple_refines_each_hole() {
    assert_eq!(
        plan_args(&["id", "slug"], Some(syn::parse_quote!((u64, String)))),
        ["id: u64", "slug: String"]
    );
}

#[test]
fn unmappable_path_type_falls_back_to_string_params() {
    assert_eq!(
        plan_args(&["id", "child"], Some(syn::parse_quote!(SomeStruct))),
        [
            "id: :: std :: string :: String",
            "child: :: std :: string :: String"
        ]
    );
}

#[test]
fn tuple_elems_recovers_elements() {
    assert_eq!(
        tuple_elems(&syn::parse_quote!((u64, String)))
            .unwrap()
            .len(),
        2
    );
    assert!(tuple_elems(&syn::parse_quote!(u64)).is_none());
}

#[test]
fn params_become_positional_placeholders() {
    let (fmt, holes) = parse_template("/users/{id}/posts/{slug}");
    assert_eq!(fmt, "{}/users/{}/posts/{}");
    assert_eq!(holes, ["id", "slug"]);
}

#[test]
fn static_route_has_no_holes() {
    let (fmt, holes) = parse_template("/health");
    assert_eq!(fmt, "{}/health");
    assert!(holes.is_empty());
}

#[test]
fn catch_all_is_one_hole() {
    let (fmt, holes) = parse_template("/files/{*path}");
    assert_eq!(fmt, "{}/files/{}");
    assert_eq!(holes, ["*path"]);
}

#[test]
fn escaped_braces_are_literal() {
    let (fmt, holes) = parse_template("/lit/{{x}}/{id}");
    assert_eq!(fmt, "{}/lit/{{x}}/{}");
    assert_eq!(holes, ["id"]);
}

#[test]
fn keyword_holes_fall_back_to_generated_param_name() {
    assert_eq!(hole_ident("type", 0).to_string(), "path0");
    assert_eq!(hole_ident("self", 1).to_string(), "path1");
}
