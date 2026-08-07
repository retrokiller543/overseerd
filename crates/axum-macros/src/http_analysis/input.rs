use overseerd_macros_core::attr::{first_type_arg, type_name};
use syn::{Ident, Type};

pub(super) fn classify(ty: &Type) -> (&'static str, Type) {
    let ty = peel_combinator(ty);

    for (wrapper, source) in [
        ("Path", "Path"),
        ("Query", "Query"),
        ("TypedHeader", "Header"),
        ("Json", "JsonBody"),
        ("TypedJson", "JsonBody"),
        ("JsonDeserializer", "JsonBody"),
        ("Protobuf", "BytesBody"),
        ("Form", "FormBody"),
        ("Inject", "Injected"),
        ("Extension", "Context"),
    ] {
        if let Some(inner) = first_type_arg(ty, wrapper) {
            return (source, inner);
        }
    }

    let source = match type_name(ty).map(Ident::to_string).as_deref() {
        Some("HeaderMap") => "Header",
        Some("Bytes") => "BytesBody",
        Some("RawForm") => "RawFormBody",
        Some("Multipart") => "MultipartBody",
        Some("JsonLines") => "Stream",
        _ => "Context",
    };

    (source, ty.clone())
}

fn peel_combinator(ty: &Type) -> &Type {
    let mut current = ty;

    loop {
        let next = match type_name(current).map(Ident::to_string).as_deref() {
            Some("Cached" | "WithRejection") => first_type_arg_ref(current),
            _ => None,
        };

        match next {
            Some(inner) => current = inner,
            None => return current,
        }
    }
}

fn first_type_arg_ref(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };

    arguments.args.iter().find_map(|argument| match argument {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}
