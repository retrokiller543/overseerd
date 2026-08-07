use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, Type};

const MAX_NAMED_PATH_PARAMS: usize = 3;

pub(super) struct PathPlan {
    pub(super) args: Vec<(Ident, TokenStream)>,
    pub(super) subst: Vec<TokenStream>,
}

pub(crate) fn hole_param_types(holes: &[String], path_ty: Option<Type>) -> Vec<Type> {
    let string_per_hole = || {
        holes
            .iter()
            .map(|_| syn::parse_quote!(::std::string::String))
            .collect()
    };
    let Some(path_ty) = path_ty else {
        return string_per_hole();
    };

    if holes.len() == 1 {
        return vec![path_ty];
    }

    match tuple_elems(&path_ty) {
        Some(types) if types.len() == holes.len() => types,
        _ => string_per_hole(),
    }
}

pub(super) fn plan_path(holes: &[String], path_ty: Option<Type>) -> PathPlan {
    if holes.is_empty() {
        return PathPlan {
            args: Vec::new(),
            subst: Vec::new(),
        };
    }

    let types = hole_param_types(holes, path_ty);

    if holes.len() <= MAX_NAMED_PATH_PARAMS {
        let args = holes
            .iter()
            .zip(&types)
            .enumerate()
            .map(|(i, (hole, ty))| (hole_ident(hole, i), quote!(#ty)))
            .collect::<Vec<_>>();
        let subst = args.iter().map(|(name, _)| quote!(#name)).collect();

        return PathPlan { args, subst };
    }

    let subst = (0..holes.len())
        .map(syn::Index::from)
        .map(|idx| quote!(path.#idx))
        .collect();
    let tuple_ty: Type = syn::parse_quote!((#(#types,)*));

    PathPlan {
        args: vec![(format_ident!("path"), quote!(#tuple_ty))],
        subst,
    }
}

pub(super) fn tuple_elems(ty: &Type) -> Option<Vec<Type>> {
    match ty {
        Type::Tuple(tuple) => Some(tuple.elems.iter().cloned().collect()),
        _ => None,
    }
}

pub(crate) fn hole_ident(hole: &str, index: usize) -> Ident {
    let name = hole.trim_start_matches('*');

    if !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !name.chars().next().is_some_and(|c| c.is_numeric())
        && syn::parse_str::<Ident>(name).is_ok()
    {
        format_ident!("{}", name)
    } else {
        format_ident!("path{}", index)
    }
}

pub(crate) fn parse_template(template: &str) -> (String, Vec<String>) {
    let mut out = String::from("{}");
    let mut holes = Vec::new();
    let mut chars = template.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push_str("{{");
            }
            '{' => {
                let mut name = String::new();

                for inner in chars.by_ref() {
                    if inner == '}' {
                        break;
                    }
                    name.push(inner);
                }

                out.push_str("{}");
                holes.push(name);
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push_str("}}");
            }
            '}' => out.push_str("}}"),
            _ => out.push(c),
        }
    }

    (out, holes)
}

#[cfg(test)]
mod tests;
