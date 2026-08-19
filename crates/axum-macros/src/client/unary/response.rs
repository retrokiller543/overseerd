use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, ReturnType, Type};
use upwell_macros_core::paths::Paths;

use super::super::response::response_type;
use crate::http_analysis::{ResponseBody, ResponseOrigin};
use crate::route::RouteAttr;

pub(super) struct ResponsePlan {
    pub(super) ty: Type,
    pub(super) declaration: TokenStream,
    pub(super) bounds: TokenStream,
    pub(super) ret: TokenStream,
    pub(super) body: TokenStream,
}

pub(super) struct ResponsePlanInput<'a> {
    pub method: &'a Ident,
    pub controller: &'a Ident,
    pub route: &'a RouteAttr,
    pub output: &'a ReturnType,
    pub contract: &'a crate::http_analysis::ResponseContract,
    pub encode_ty: &'a TokenStream,
    pub request: &'a TokenStream,
    pub paths: &'a Paths,
}

pub(super) fn response_plan(input: ResponsePlanInput<'_>) -> ResponsePlan {
    let ResponsePlanInput {
        method,
        controller,
        route,
        output,
        contract,
        encode_ty,
        request,
        paths,
    } = input;
    let http = paths.plugin("http");
    let http_exchange = paths.plugin("client::HttpExchange");
    let http_response = paths.plugin("client::HttpResponse");
    let encodes = paths.plugin("client::Encodes");
    let decodes = paths.plugin("client::Decodes");
    let client_error = paths.core("client::ClientError");
    let conventional = route
        .returns
        .clone()
        .unwrap_or_else(|| response_type(output));
    let alternatives = &contract.alternatives;
    let raw: Type = syn::parse_quote!(::std::vec::Vec<u8>);
    let common = common_shape(alternatives);

    let (ty, declaration, constructors) = if let Some(user_response) = &route.response {
        let constructors = alternatives
            .iter()
            .map(|response| match response.body {
                ResponseBody::Typed(_) => {
                    quote!(::core::convert::Into::<#user_response>::into(__decoded))
                }
                ResponseBody::Empty if response.redirect.is_some() => {
                    let redirect = paths.plugin("client::RedirectResponse");
                    quote!(::core::convert::Into::<#user_response>::into(
                        #redirect::from_parts(__status, &__headers)
                    ))
                }
                ResponseBody::Empty => quote!(::core::convert::Into::<#user_response>::into(())),
                ResponseBody::Opaque => {
                    quote!(::core::convert::Into::<#user_response>::into(__bytes))
                }
            })
            .collect();

        (user_response.clone(), quote!(), constructors)
    } else if let Some((common, redirecting)) = common {
        let constructor = match &common {
            ResponseBody::Typed(_) => quote!(__decoded),
            ResponseBody::Empty if redirecting => {
                let redirect = paths.plugin("client::RedirectResponse");
                quote!(#redirect::from_parts(__status, &__headers))
            }
            ResponseBody::Empty => quote!(()),
            ResponseBody::Opaque => quote!(__bytes),
        };
        let ty = match common {
            ResponseBody::Typed(ty) => *ty,
            ResponseBody::Empty if redirecting => {
                let redirect = paths.plugin("client::RedirectResponse");
                syn::parse_quote!(#redirect)
            }
            ResponseBody::Empty => syn::parse_quote!(()),
            ResponseBody::Opaque => raw.clone(),
        };

        (
            ty,
            quote!(),
            alternatives
                .iter()
                .map(|_| constructor.clone())
                .collect::<Vec<_>>(),
        )
    } else if contract.origin == ResponseOrigin::Conventional {
        (
            conventional,
            quote!(),
            alternatives
                .iter()
                .map(|_| quote!(__decoded))
                .collect::<Vec<_>>(),
        )
    } else if alternatives.is_empty() {
        (raw, quote!(), Vec::new())
    } else {
        let ident = format_ident!("{}{}Response", controller, upper_camel(&method.to_string()));
        let variants = alternatives.iter().map(|response| {
            let variant = format_ident!("Status{}", response.status);

            match &response.body {
                ResponseBody::Typed(body) => quote!(#variant(#body)),
                ResponseBody::Empty if response.redirect.is_some() => {
                    let redirect = paths.plugin("client::RedirectResponse");
                    quote!(#variant(#redirect))
                }
                ResponseBody::Empty => quote!(#variant),
                ResponseBody::Opaque => quote!(#variant(::std::vec::Vec<u8>)),
            }
        });
        let constructors = alternatives
            .iter()
            .map(|response| {
                let variant = format_ident!("Status{}", response.status);

                match response.body {
                    ResponseBody::Typed(_) => quote!(#ident::#variant(__decoded)),
                    ResponseBody::Empty if response.redirect.is_some() => {
                        let redirect = paths.plugin("client::RedirectResponse");
                        quote!(#ident::#variant(#redirect::from_parts(__status, &__headers)))
                    }
                    ResponseBody::Empty => quote!(#ident::#variant),
                    ResponseBody::Opaque => quote!(#ident::#variant(__bytes)),
                }
            })
            .collect();
        let dto = paths.plugin("dto");

        (
            syn::parse_quote!(#ident),
            quote! {
                #[#dto]
                pub enum #ident {
                    #(#variants),*
                }
            },
            constructors,
        )
    };
    let decode_bounds = alternatives
        .iter()
        .filter_map(|response| match &response.body {
            ResponseBody::Typed(body) => Some(quote!(+ #decodes<#body>)),
            _ => None,
        });
    let conversion_bounds = route
        .response
        .as_ref()
        .into_iter()
        .flat_map(|response| {
            alternatives
                .iter()
                .map(move |alternative| match &alternative.body {
                    ResponseBody::Typed(body) => quote!(#response: ::core::convert::From<#body>),
                    ResponseBody::Empty if alternative.redirect.is_some() => {
                        let redirect = paths.plugin("client::RedirectResponse");
                        quote!(#response: ::core::convert::From<#redirect>)
                    }
                    ResponseBody::Empty => quote!(#response: ::core::convert::From<()>),
                    ResponseBody::Opaque => {
                        quote!(#response: ::core::convert::From<::std::vec::Vec<u8>>)
                    }
                })
        })
        .collect::<Vec<_>>();
    let conversion_where = if conversion_bounds.is_empty() {
        quote!()
    } else {
        quote!(, #(#conversion_bounds),*)
    };
    let decode_arms = alternatives
        .iter()
        .zip(constructors)
        .map(|(response, constructor)| {
            let status = response.status;

            match &response.body {
                ResponseBody::Typed(body) => quote! {
                    #status => {
                        let (__status, __headers, __bytes) = __response.into_parts();
                        let __decoded = <C as #http_exchange>::decode_response::<#body>(
                            &self.0,
                            __bytes,
                        )?;
                        ::core::result::Result::Ok(#http_response::new(
                            __status,
                            __headers,
                            #constructor,
                        ))
                    }
                },
                ResponseBody::Empty => quote! {
                    #status => {
                        let (__status, __headers, _) = __response.into_parts();
                        let __body = #constructor;
                        ::core::result::Result::Ok(#http_response::new(
                            __status,
                            __headers,
                            __body,
                        ))
                    }
                },
                ResponseBody::Opaque => quote! {
                    #status => {
                        let (__status, __headers, __bytes) = __response.into_parts();
                        ::core::result::Result::Ok(#http_response::new(
                            __status,
                            __headers,
                            #constructor,
                        ))
                    }
                },
            }
        });
    let response_body = if alternatives.is_empty() {
        quote! {
            <C as #http_exchange>::exchange(&self.0, #request).await
        }
    } else {
        quote! {
            let __response = <C as #http_exchange>::exchange(&self.0, #request).await?;
            match __response.status().as_u16() {
                #(#decode_arms)*
                _ => ::core::result::Result::Err(
                    <C as #http_exchange>::fail_unexpected(&self.0, __response)
                ),
            }
        }
    };

    ResponsePlan {
        ty: ty.clone(),
        declaration,
        bounds: quote!(C: #http_exchange + #encodes<#encode_ty> #(#decode_bounds)* #conversion_where),
        ret: quote!(::core::result::Result<#http_response<#ty>, #client_error<#http::StatusCode>>),
        body: response_body,
    }
}

fn common_shape(
    alternatives: &[crate::http_analysis::ResponseAlternative],
) -> Option<(ResponseBody, bool)> {
    let first = alternatives.first()?;
    let body = first.body.clone();
    let redirecting = first.redirect.is_some();

    alternatives
        .iter()
        .all(|alternative| {
            same_body(&body, &alternative.body) && redirecting == alternative.redirect.is_some()
        })
        .then_some((body, redirecting))
}

fn same_body(left: &ResponseBody, right: &ResponseBody) -> bool {
    match (left, right) {
        (ResponseBody::Empty, ResponseBody::Empty)
        | (ResponseBody::Opaque, ResponseBody::Opaque) => true,
        (ResponseBody::Typed(left), ResponseBody::Typed(right)) => {
            quote!(#left).to_string() == quote!(#right).to_string()
        }
        _ => false,
    }
}

fn upper_camel(value: &str) -> String {
    value
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut characters = segment.chars();
            characters
                .next()
                .map(char::to_uppercase)
                .into_iter()
                .flatten()
                .chain(characters)
                .collect::<String>()
        })
        .collect()
}
