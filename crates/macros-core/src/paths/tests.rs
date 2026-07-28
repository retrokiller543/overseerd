use std::str::FromStr as _;

use proc_macro2::TokenStream;
use quote::ToTokens as _;
use syn::Path;

use super::Paths;

#[test]
fn joined_paths_preserve_user_root_segments_and_locations() {
    let root_tokens =
        TokenStream::from_str("::renamed_facade::private_root").expect("root token stream parses");
    let root = syn::parse2::<Path>(root_tokens).expect("root path parses");
    let paths = Paths::new(root.clone(), syn::parse_quote!(::plugin));
    let joined = paths.core("client::Client");

    assert!(joined.leading_colon.is_some());
    assert_eq!(
        joined.to_token_stream().to_string(),
        ":: renamed_facade :: private_root :: client :: Client"
    );

    for (original, preserved) in root.segments.iter().zip(&joined.segments) {
        assert_eq!(
            original.ident.span().start(),
            preserved.ident.span().start()
        );
        assert_eq!(original.ident.span().end(), preserved.ident.span().end());
    }
}

#[test]
fn joined_paths_keep_relative_roots_relative() {
    let root = syn::parse_str::<Path>("crate::renamed").expect("relative root parses");
    let joined = Paths::new(root, syn::parse_quote!(::plugin)).core("App");

    assert!(joined.leading_colon.is_none());
    assert_eq!(
        joined.to_token_stream().to_string(),
        "crate :: renamed :: App"
    );
}

#[test]
fn syn_paths_reject_qualified_self_roots() {
    let error = match syn::parse_str::<Path>("<T as Trait>::Associated") {
        Ok(_) => panic!("qualified-self syntax unexpectedly parsed as a syn::Path root"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("expected identifier"));
}
