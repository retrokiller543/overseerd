use overseerd_macros_core::attr::{first_type_arg, type_name};
use syn::{Ident, ReturnType, Type};

pub(crate) fn response_type(output: &ReturnType) -> Type {
    let declared = match output {
        ReturnType::Type(_, ty) => (**ty).clone(),
        ReturnType::Default => return syn::parse_quote!(()),
    };

    response_body_type(&declared).unwrap_or(declared)
}

fn response_body_type(ty: &Type) -> Option<Type> {
    let inner = first_type_arg(ty, "Result").unwrap_or_else(|| ty.clone());

    if let Type::Tuple(tuple) = &inner {
        return match tuple.elems.last() {
            Some(body) => response_body_type(body),
            None => Some(inner),
        };
    }

    first_type_arg(&inner, "Json").or_else(|| {
        (!is_opaque_response(&inner)
            && !matches!(
                type_name(&inner).map(Ident::to_string).as_deref(),
                Some("Html" | "Redirect")
            ))
        .then_some(inner)
    })
}

pub(crate) fn is_opaque_response(ty: &Type) -> bool {
    match ty {
        Type::ImplTrait(_) => true,
        Type::Path(path) => path.path.segments.last().is_some_and(|segment| {
            matches!(
                segment.ident.to_string().as_str(),
                "Response" | "Html" | "Redirect"
            )
        }),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
