use proc_macro2::TokenStream;
use quote::quote;
use syn::{GenericArgument, PathArguments, ReturnType, Type, TypeParamBound};
use upwell_macros_core::attr::{first_type_arg, type_name};
use upwell_macros_core::paths::Paths;

/// How the macro wraps a bare stream return server-side.
pub(crate) enum ServerWrap {
    Ndjson,
    RawU8,
}

/// A classified server-streaming return.
pub(crate) struct StreamReturn {
    pub(crate) server_wrap: Option<ServerWrap>,
    pub(crate) in_result: bool,
    pub(crate) client: Option<(TokenStream, Type)>,
}

/// Classifies built-in stream wrappers, bare streams, and explicitly opaque streamed returns.
pub(crate) fn classify_stream_return(
    output: &ReturnType,
    streamed: bool,
    paths: &Paths,
) -> Option<StreamReturn> {
    let ReturnType::Type(_, ty) = output else {
        return None;
    };

    let (inner, in_result) = match first_type_arg(ty, "Result") {
        Some(ok) => (ok, true),
        None => ((**ty).clone(), false),
    };

    if let Type::Path(type_path) = &inner
        && let Some(segment) = type_path.path.segments.last()
        && matches!(segment.ident.to_string().as_str(), "Ndjson" | "RawStream")
        && let PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(GenericArgument::Type(stream_ty)) = args.args.first()
    {
        let mut bare = type_path.clone();
        bare.path.segments.last_mut()?.arguments = PathArguments::None;

        return Some(StreamReturn {
            server_wrap: None,
            in_result,
            client: Some((quote!(#bare<()>), stream_item(stream_ty, paths)?)),
        });
    }

    if let Type::ImplTrait(impl_trait) = &inner
        && let Some(item) = stream_item_binding(impl_trait)
    {
        let ndjson = paths.plugin("Ndjson");
        let raw = paths.plugin("RawStream");

        if type_name(&item).is_some_and(|name| name == "u8") {
            let bytes = paths.plugin("bytes::Bytes");
            return Some(StreamReturn {
                server_wrap: Some(ServerWrap::RawU8),
                in_result,
                client: Some((quote!(#raw<()>), syn::parse_quote!(#bytes))),
            });
        }

        return Some(StreamReturn {
            server_wrap: Some(ServerWrap::Ndjson),
            in_result,
            client: Some((quote!(#ndjson<()>), item)),
        });
    }

    streamed.then_some(StreamReturn {
        server_wrap: None,
        in_result,
        client: None,
    })
}

/// Returns a stream's item type from an `impl Stream` binding or concrete projection.
pub(crate) fn stream_item(ty: &Type, paths: &Paths) -> Option<Type> {
    match ty {
        Type::ImplTrait(impl_trait) => stream_item_binding(impl_trait),
        concrete => {
            let stream = paths.plugin("__Stream");
            Some(syn::parse_quote!(<#concrete as #stream>::Item))
        }
    }
}

fn stream_item_binding(impl_trait: &syn::TypeImplTrait) -> Option<Type> {
    for bound in &impl_trait.bounds {
        let TypeParamBound::Trait(trait_bound) = bound else {
            continue;
        };
        let segment = trait_bound.path.segments.last()?;

        if segment.ident != "Stream" {
            continue;
        }

        let PathArguments::AngleBracketed(args) = &segment.arguments else {
            continue;
        };

        for arg in &args.args {
            if let GenericArgument::AssocType(assoc) = arg
                && assoc.ident == "Item"
            {
                return Some(assoc.ty.clone());
            }
        }
    }

    None
}
