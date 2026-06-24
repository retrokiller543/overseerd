//! Source-level parsing: walk a crate's `.rs` files and reconstruct the DI graph
//! by name. This sees source *as written* (not macro-expanded), so it is
//! intra-crate and name-based — the documented soundness boundary of build-time
//! analysis. A dependency provided at runtime must therefore live in the same
//! crate (carrying a `Component`-ish attribute) or be marked `Dynamic`.

use std::fs;
use std::path::{Path, PathBuf};

use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Expr, ExprLit, Field, FnArg, GenericArgument, ImplItem, Item, ItemImpl, ItemStruct,
    Lit, Meta, PathArguments, Token, Type, TypeParamBound,
};

/// The reconstructed graph for a crate: the raw structs and `#[handlers]` impls,
/// from which [`crate::check`] derives components, providers, services, and RPCs.
#[derive(Default)]
pub struct Model {
    pub structs: Vec<Struct>,
    pub impls: Vec<Handlers>,
}

/// A struct carrying a `Component`-ish attribute.
pub struct Struct {
    pub name: String,
    pub id: String,
    pub display_name: String,
    pub injected: bool,
    pub is_service: bool,
    pub field_deps: Vec<Dependency>,
    pub provides: Vec<Provided>,
    pub file: PathBuf,
    pub line: usize,
}

/// A trait this struct provides (`provide = dyn Trait`), with its resolved
/// qualifier and whether it is the primary provider.
pub struct Provided {
    pub trait_name: String,
    pub qualifier: String,
    pub primary: bool,
}

/// A required, single-valued dependency — concrete (`Arc<T>` / by-value `T`) or a
/// trait object (`Arc<dyn Trait>`). Optional / `Dynamic` / collection edges are
/// not recorded (they can never be "missing").
pub struct Dependency {
    pub name: String,
    pub is_trait: bool,
    pub qualifier: Option<String>,
    pub line: usize,
}

/// A `#[handlers] impl T` block.
pub struct Handlers {
    pub self_name: String,
    /// Dependencies from an `#[init]` constructor's parameters; `Some` only when
    /// the block has an `#[init]`, in which case these override field injection.
    pub init_deps: Option<Vec<Dependency>>,
    pub rpcs: Vec<Rpc>,
    pub file: PathBuf,
}

/// An `#[rpc]` method.
pub struct Rpc {
    pub name: String,
    pub line: usize,
}

impl Model {
    /// Parses every `.rs` file under `dir`, merging each into one model. Files
    /// that fail to parse are skipped (rustc reports the real syntax error).
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

    /// Parses one source string, attributing items to `file`.
    pub fn add_source(&mut self, source: &str, file: &Path) {
        let Ok(parsed) = syn::parse_file(source) else {
            return;
        };

        for item in &parsed.items {
            match item {
                Item::Struct(item) => {
                    if let Some(kind) = component_kind(&item.attrs) {
                        self.structs.push(struct_of(item, kind, file));
                    }
                }

                Item::Impl(item) if has_attr(&item.attrs, "handlers") => {
                    if let Some(handlers) = handlers_of(item, file) {
                        self.impls.push(handlers);
                    }
                }

                _ => {}
            }
        }
    }
}

/// How a struct participates: a `#[component]` or a `#[service]`. Whether it is
/// field-injected or manually provided is read from its arguments (`manual`).
enum Kind {
    Component,
    Service,
}

fn struct_of(item: &ItemStruct, kind: Kind, file: &Path) -> Struct {
    let name = item.ident.to_string();
    let is_service = matches!(kind, Kind::Service);

    let attr_name = if is_service { "service" } else { "component" };
    let args = item
        .attrs
        .iter()
        .find(|a| a.path().is_ident(attr_name))
        .map(ComponentArgs::from_attr)
        .unwrap_or_default();

    // A manual / explicit-factory component does not field-inject: its struct fields
    // are not the injected dependencies.
    let injected = !args.manual;

    let id = args.id.clone().unwrap_or_else(|| name.to_lowercase());
    let display_name = args.display_name.clone().unwrap_or_else(|| name.clone());
    let qualifier = args.qualifier.clone().unwrap_or_else(|| id.clone());

    let provides = args
        .provide
        .iter()
        .map(|trait_name| Provided {
            trait_name: trait_name.clone(),
            qualifier: qualifier.clone(),
            primary: args.primary,
        })
        .collect();

    let field_deps = if injected {
        item.fields.iter().filter_map(field_dependency).collect()
    } else {
        Vec::new()
    };

    Struct {
        name,
        id,
        display_name,
        injected,
        is_service,
        field_deps,
        provides,
        file: file.to_path_buf(),
        line: item.ident.span().start().line,
    }
}

fn handlers_of(item: &ItemImpl, file: &Path) -> Option<Handlers> {
    let self_name = type_name(&item.self_ty)?;
    let mut init_deps = None;
    let mut rpcs = Vec::new();

    for impl_item in &item.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };

        if has_attr(&method.attrs, "init") {
            let deps = method
                .sig
                .inputs
                .iter()
                .filter_map(|arg| match arg {
                    FnArg::Typed(typed) => dependency_of_type(&typed.ty, None),
                    FnArg::Receiver(_) => None,
                })
                .collect();

            init_deps = Some(deps);
        } else if has_attr(&method.attrs, "rpc") {
            rpcs.push(Rpc {
                name: method.sig.ident.to_string(),
                line: method.sig.ident.span().start().line,
            });
        }
    }

    Some(Handlers {
        self_name,
        init_deps,
        rpcs,
        file: file.to_path_buf(),
    })
}

/// The required dependency a field expresses, if any (carrying a `#[qualifier]`).
fn field_dependency(field: &Field) -> Option<Dependency> {
    if has_attr(&field.attrs, "default") {
        return None;
    }

    let qualifier = field.attrs.iter().find_map(field_qualifier);

    dependency_of_type(&field.ty, qualifier)
}

/// The qualifier string of a `#[qualifier = ".."]` attribute.
fn field_qualifier(attr: &Attribute) -> Option<String> {
    if !attr.path().is_ident("qualifier") {
        return None;
    }

    if let Meta::NameValue(nv) = &attr.meta
        && let Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) = &nv.value
    {
        return Some(s.value());
    }

    None
}

/// Classifies a type into a required single dependency: `Arc<T>` → concrete `T`,
/// `Arc<dyn Trait>` → trait `Trait`, by-value `T` → concrete `T`. `None` for
/// `Option`/`Dynamic`/`Vec`/`HashMap` (never missing) and bare trait objects.
fn dependency_of_type(ty: &Type, qualifier: Option<String>) -> Option<Dependency> {
    if matches!(ty, Type::TraitObject(_)) {
        return None;
    }

    let segment = last_segment(ty)?;
    let line = ty.span().start().line;

    match segment.as_str() {
        "Option" | "Dynamic" | "Vec" | "HashMap" => None,

        "Arc" => {
            let inner = first_generic(ty)?;

            if let Type::TraitObject(_) = inner {
                Some(Dependency {
                    name: trait_name(inner)?,
                    is_trait: true,
                    qualifier,
                    line,
                })
            } else {
                Some(Dependency {
                    name: last_segment(inner)?,
                    is_trait: false,
                    qualifier,
                    line,
                })
            }
        }

        _ => Some(Dependency {
            name: segment,
            is_trait: false,
            qualifier,
            line,
        }),
    }
}

/// Parsed `#[component(..)]` / `#[service(..)]` arguments (lenient: a parse
/// failure yields defaults, since rustc reports the real error).
#[derive(Default)]
struct ComponentArgs {
    id: Option<String>,
    display_name: Option<String>,
    qualifier: Option<String>,
    primary: bool,
    provide: Vec<String>,
    /// Opts out of field injection — `default_factory = false` (a manual instance)
    /// or an explicit `factory = path` (deps come from the factory's parameters).
    manual: bool,
}

impl ComponentArgs {
    fn from_attr(attr: &Attribute) -> Self {
        attr.parse_args_with(parse_component_args)
            .unwrap_or_default()
    }
}

/// Parses the comma-separated argument list inside `#[component(..)]`.
fn parse_component_args(input: syn::parse::ParseStream) -> syn::Result<ComponentArgs> {
    let mut args = ComponentArgs::default();

    while !input.is_empty() {
        let key: syn::Ident = input.parse()?;

        match key.to_string().as_str() {
            "id" => args.id = Some(parse_str_value(input)?),
            "name" => args.display_name = Some(parse_str_value(input)?),
            "qualifier" => args.qualifier = Some(parse_str_value(input)?),
            "version" => {
                let _ = parse_str_value(input)?;
            }
            "primary" => args.primary = true,
            "by_value" => {}
            "factory" => {
                input.parse::<Token![=]>()?;
                let _ = input.parse::<syn::Path>()?;
                args.manual = true;
            }
            "default_factory" => {
                input.parse::<Token![=]>()?;
                let value: syn::LitBool = input.parse()?;
                args.manual = !value.value;
            }
            "scope" | "rpc_slice" | "factory_slice" => {
                input.parse::<Token![=]>()?;
                let _ = input.parse::<syn::Ident>()?;
            }
            "provide" => {
                input.parse::<Token![=]>()?;

                if input.peek(syn::token::Bracket) {
                    let content;
                    syn::bracketed!(content in input);
                    let list = Punctuated::<Type, Token![,]>::parse_terminated(&content)?;

                    for ty in list {
                        if let Some(name) = trait_name(&ty) {
                            args.provide.push(name);
                        }
                    }
                } else if let Some(name) = trait_name(&input.parse::<Type>()?) {
                    args.provide.push(name);
                }
            }
            _ => return Ok(args),
        }

        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
    }

    Ok(args)
}

fn parse_str_value(input: syn::parse::ParseStream) -> syn::Result<String> {
    input.parse::<Token![=]>()?;
    let lit: syn::LitStr = input.parse()?;

    Ok(lit.value())
}

fn component_kind(attrs: &[Attribute]) -> Option<Kind> {
    for attr in attrs {
        if attr.path().is_ident("service") {
            return Some(Kind::Service);
        }

        if attr.path().is_ident("component") {
            return Some(Kind::Component);
        }
    }

    None
}

fn has_attr(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|a| a.path().is_ident(name))
}

/// The trait name of a `dyn Trait` type — the last path segment of its first
/// trait bound (`dyn a::b::Repo + Send` → `Repo`).
fn trait_name(ty: &Type) -> Option<String> {
    let Type::TraitObject(object) = ty else {
        return None;
    };

    object.bounds.iter().find_map(|bound| match bound {
        TypeParamBound::Trait(t) => t.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    })
}

/// The last path-segment ident of a named type (`a::b::Foo<..>` → `Foo`).
fn type_name(ty: &Type) -> Option<String> {
    last_segment(ty)
}

fn last_segment(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(path) => path.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}

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
