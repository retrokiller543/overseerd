use quote::ToTokens;

use super::{InjectableArgs, expand};
use crate::paths::Paths;

fn compact(tokens: impl ToTokens) -> String {
    tokens.to_token_stream().to_string().replace(' ', "")
}

#[test]
fn native_trait_gains_runtime_component_descriptor() {
    let item = syn::parse_quote! {
        pub trait Repository: Send + Sync {}
    };
    let expanded = compact(expand(item, &Paths::overseerd()));

    assert!(expanded.contains("cfg(target_family=\"wasm\")"));
    assert!(expanded.contains("traitRepository:Send+Sync{}"));
    assert!(expanded.contains("cfg(not(target_family=\"wasm\"))"));
    assert!(expanded.contains(
        "traitRepository:Send+Sync+::overseerd::RuntimeDescriptor<::overseerd::ComponentDescriptor>{}"
    ));
}

#[test]
fn runtime_descriptor_uses_overridden_facade_path() {
    let args: InjectableArgs = syn::parse_quote!(overseerd = ::framework, crate = ::plugin);
    let item = syn::parse_quote! {
        trait Repository {}
    };
    let expanded = compact(expand(item, &args.paths(Paths::overseerd())));

    assert!(expanded.contains(
        "traitRepository:::framework::RuntimeDescriptor<::framework::ComponentDescriptor>{}"
    ));
}
