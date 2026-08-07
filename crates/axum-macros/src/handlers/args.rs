use syn::{
    FnArg, GenericArgument, Ident, ImplItemFn, PathArguments, ReturnType, Type, TypeParamBound,
    parse_quote,
};

use overseerd_macros_core::paths::Paths;

use crate::client;

/// Finds and strips a `#[stream]` request-body parameter.
pub(super) fn take_stream_param(
    method: &mut ImplItemFn,
    paths: &Paths,
) -> syn::Result<Option<(usize, Type)>> {
    let mut found = None;

    for (typed_index, arg) in method
        .sig
        .inputs
        .iter_mut()
        .filter_map(|arg| match arg {
            FnArg::Typed(typed) => Some(typed),
            FnArg::Receiver(_) => None,
        })
        .enumerate()
    {
        let Some(pos) = arg.attrs.iter().position(|a| a.path().is_ident("stream")) else {
            continue;
        };

        if found.is_some() {
            return Err(syn::Error::new_spanned(
                &arg.pat,
                "a route may take at most one `#[stream]` request-body parameter",
            ));
        }

        arg.attrs.remove(pos);
        let item = client::stream_item(&arg.ty, paths).ok_or_else(|| {
            syn::Error::new_spanned(
                &arg.ty,
                "a `#[stream]` parameter must be `impl Stream<Item = T>` (or a concrete `Stream` type)",
            )
        })?;
        found = Some((typed_index, item));
    }

    Ok(found)
}

/// Injects precise capture onto a streamed `impl Trait` return.
pub(super) fn add_use_capture(output: &mut ReturnType, capture: &[Ident]) {
    if let ReturnType::Type(_, ty) = output {
        inject_capture(ty, capture);
    }
}

fn capture_impl_trait(impl_trait: &mut syn::TypeImplTrait, capture: &[Ident]) {
    let has_capture = impl_trait
        .bounds
        .iter()
        .any(|bound| matches!(bound, TypeParamBound::PreciseCapture(_)));

    if !has_capture {
        impl_trait.bounds.push(parse_quote!(use<#(#capture),*>));
    }
}

fn inject_capture(ty: &mut Type, capture: &[Ident]) {
    if let Type::ImplTrait(impl_trait) = ty {
        capture_impl_trait(impl_trait, capture);
        return;
    }

    let Type::Path(type_path) = ty else {
        return;
    };
    let Some(segment) = type_path.path.segments.last_mut() else {
        return;
    };
    let is_result = segment.ident == "Result";
    let PathArguments::AngleBracketed(args) = &mut segment.arguments else {
        return;
    };
    let Some(GenericArgument::Type(inner)) = args.args.first_mut() else {
        return;
    };

    if is_result {
        inject_capture(inner, capture);
    } else if let Type::ImplTrait(impl_trait) = inner {
        capture_impl_trait(impl_trait, capture);
    }
}
