use quote::quote;

use super::response_type;

#[test]
fn response_type_peels_result_then_json() {
    let output = syn::parse_quote!(-> Result<Json<User>, HandlerError>);
    let ty = response_type(&output);

    assert_eq!(quote!(#ty).to_string(), "User");
}
