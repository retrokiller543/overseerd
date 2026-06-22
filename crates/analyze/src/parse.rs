//! Source-level parsing: walk a crate's `.rs` files and reconstruct the DI graph
//! by name. This sees source *as written* (not macro-expanded), so it is
//! intra-crate and name-based — the documented soundness boundary of build-time
//! analysis. A dependency provided at runtime must therefore live in the same
//! crate (carrying a `Component`-ish attribute) or be marked `Dynamic`.

use std::fs;
use std::path::{Path, PathBuf};

use syn::spanned::Spanned;
use syn::{Attribute, Field, GenericArgument, Item, ItemStruct, PathArguments, Type};

/// The reconstructed component graph for a crate.
#[derive(Default)]
pub struct Model {
    pub components: Vec<Component>,
}

/// A type carrying a `Component`-ish attribute (`#[component]`, `#[service]`, or
/// `#[derive(Component)]`), with the required single-valued dependencies read off
/// its fields.
pub struct Component {
    pub name: String,
    pub deps: Vec<Dependency>,
    pub file: PathBuf,
    pub line: usize,
}

/// A required, single-valued, concrete dependency (the kind whose absence is an
/// error). Optional / `Dynamic` / collection / trait-object edges are not
/// recorded here — they cannot be "missing".
pub struct Dependency {
    pub name: String,
    pub line: usize,
}

impl Model {
    /// Parses every `.rs` file under `dir`, merging each into one model. Files
    /// that fail to parse are skipped (rustc will report the real syntax error).
    pub fn from_dir(dir: &Path) -> Self {
        let mut model = Model::default();
        let mut files = Vec::new();

        collect_rs_files(dir, &mut files);

        for file in files {
            if let Ok(source) = fs::read_to_string(&file) {
                model.add_source(&source, &file);
            }
        }

        model
    }

    /// Parses one source string, attributing components to `file`.
    pub fn add_source(&mut self, source: &str, file: &Path) {
        let Ok(parsed) = syn::parse_file(source) else {
            return;
        };

        for item in &parsed.items {
            if let Item::Struct(item) = item
                && let Some(injected) = component_kind(&item.attrs)
            {
                self.components.push(component_of(item, injected, file));
            }
        }
    }
}

/// Recursively gathers `.rs` files under `dir`.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Classifies a struct's attributes: `Some(true)` for a field-injected component
/// (`#[component]`/`#[service]`), `Some(false)` for a metadata-only
/// `#[derive(Component)]` (manually provided — its fields are *not* injected),
/// and `None` if it is not a component at all.
fn component_kind(attrs: &[Attribute]) -> Option<bool> {
    for attr in attrs {
        if attr.path().is_ident("component") || attr.path().is_ident("service") {
            return Some(true);
        }

        if attr.path().is_ident("derive") {
            let mut derives_component = false;

            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("Component") {
                    derives_component = true;
                }

                Ok(())
            });

            if derives_component {
                return Some(false);
            }
        }
    }

    None
}

/// Builds a [`Component`] from a struct. Only field-injected components read deps
/// from their fields; a manually-provided `#[derive(Component)]` has none.
fn component_of(item: &ItemStruct, injected: bool, file: &Path) -> Component {
    let deps = if injected {
        item.fields.iter().filter_map(field_dependency).collect()
    } else {
        Vec::new()
    };

    Component {
        name: item.ident.to_string(),
        deps,
        file: file.to_path_buf(),
        line: item.ident.span().start().line,
    }
}

/// The required concrete dependency a field expresses, if any. Returns `None` for
/// `#[default]` local state, and for optional / `Dynamic` / collection /
/// trait-object edges (none of which can be "missing").
fn field_dependency(field: &Field) -> Option<Dependency> {
    if field.attrs.iter().any(|a| a.path().is_ident("default")) {
        return None;
    }

    let name = concrete_dep_name(&field.ty)?;

    Some(Dependency {
        name,
        line: field.ty.span().start().line,
    })
}

/// The concrete type name a field resolves to as a required single dependency:
/// `Arc<T>` → `T`, or a by-value `T` → `T`. `None` for wrappers that are not
/// required-single (`Option`/`Dynamic`/`Vec`/`HashMap`) or for trait objects
/// (whose providers need cross-item analysis, deferred).
fn concrete_dep_name(ty: &Type) -> Option<String> {
    if matches!(ty, Type::TraitObject(_)) {
        return None;
    }

    let segment = last_segment(ty)?;

    match segment.as_str() {
        "Option" | "Dynamic" | "Vec" | "HashMap" => None,

        "Arc" => {
            let inner = first_generic(ty)?;

            if matches!(inner, Type::TraitObject(_)) {
                return None;
            }

            last_segment(inner)
        }

        _ => Some(segment),
    }
}

/// The last path-segment ident of a path type (`a::b::Foo<..>` → `Foo`).
fn last_segment(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(path) => path.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}

/// The first generic type argument of `Name<T, ..>`.
fn first_generic(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;

    let PathArguments::AngleBracketed(generics) = &segment.arguments else {
        return None;
    };

    generics.args.iter().find_map(|arg| match arg {
        GenericArgument::Type(t) => Some(t),
        _ => None,
    })
}