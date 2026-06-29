//! The macro extension seams.
//!
//! Overseerd's attribute macros are built as a **state machine** over an extension value. The
//! extension starts at [`Default`] and is driven through single-purpose phases, each mutating
//! its accumulated state; [`Expand::expand`] then reads that state. This lets `#[service]` be
//! `#[component]` + a router extension, and `#[handlers]` be `#[methods]` + an RPC extension,
//! with no forked parsers.
//!
//! The phases (each its own trait, so an extension implements only what it needs):
//!
//! 1. [`ParseKeyed`] — the macro-invocation keyed args (`#[m(key = value, ..)]`). The base arg
//!    type ([`ComponentArgs`](crate::ComponentArgs) / [`MethodArgs`](crate::MethodArgs)) parses
//!    its own common keys and hands every unknown key to the extension.
//! 2. [`ParseItem<T>`] — an optional first pass over the whole annotated item (the `ItemImpl`,
//!    or the struct/enum `DeriveInput`), to capture context (the type's name, …) or analyze it.
//! 3. [`ParseMethod`] — for impl macros, one pass per method, updating state and stripping the
//!    extension's marker attribute (e.g. `#[rpc]`).
//! 4. [`ToTokens`] — emit the accumulated contribution, appended after the base output. The
//!    earlier phases captured everything into the extension, so emission needs only `&self`,
//!    which is exactly what `ToTokens` provides. (This is also the "user-controlled control"
//!    seam: the base owns the skeleton and only splices the extension's tokens at a fixed
//!    point — the extension adds, it never rewrites.)
//!
//! [`NoExt`] is the single no-op default for every slot, so `ComponentArgs<NoExt>` is exactly
//! `#[component]` and `MethodArgs<NoExt>` is exactly `#[methods]`.

use proc_macro2::TokenStream;
use quote::ToTokens;
use syn::parse::ParseStream;
use syn::{Ident, ImplItemFn, Token};

/// Phase 1 — parse the macro-invocation keyed args, a state machine over `key = value` (or
/// bare-flag) entries. The driving arg type reads each key ident, tries its own common keys,
/// and otherwise calls [`parse_keyed`](Self::parse_keyed) with the already-read key.
///
/// [`ToTokens`] is a supertrait: it is phase 4, the emission of the accumulated state.
pub trait ParseKeyed: Default + ToTokens {
    /// Parse the value for `key` (the ident is already consumed) from `input`.
    ///
    /// - `Ok(true)` — recognized; its value (or, for a bare flag, nothing) is consumed.
    /// - `Ok(false)` — unknown; `input` is left untouched so the base errors.
    /// - `Err(..)` — recognized but malformed (spanned at the value).
    fn parse_keyed(&mut self, key: &Ident, input: ParseStream) -> syn::Result<bool> {
        let _ = (key, input);

        Ok(false)
    }

    /// The keys this extension recognizes, for the "unknown argument" diagnostic.
    fn expected_keys() -> &'static [&'static str] {
        &[]
    }
}

/// Phase 2 (optional) — a first pass over the whole annotated item `T` (an `ItemImpl`, or a
/// struct/enum `DeriveInput`), to capture context the later phases and [`Expand`] need (the
/// type ident, its name, generics, …) before per-method parsing.
pub trait ParseItem<T>: Default {
    fn parse_item(&mut self, item: &T) -> syn::Result<()> {
        let _ = item;

        Ok(())
    }
}

/// Phase 3 (impl macros) — one pass per method. Inspect the method, update accumulated state,
/// and strip the extension's own marker attribute if present (the base has already claimed and
/// stripped `#[init]`/`#[hook]`).
pub trait ParseMethod: Default {
    fn parse_method(&mut self, method: &mut ImplItemFn) -> syn::Result<()> {
        let _ = method;

        Ok(())
    }
}

/// The single no-op extension: the default for every slot. `ComponentArgs<NoExt>` is exactly
/// `#[component]`; `MethodArgs<NoExt>` is exactly `#[methods]`. Emits nothing.
#[derive(Default)]
pub struct NoExt;

impl ToTokens for NoExt {
    fn to_tokens(&self, _tokens: &mut TokenStream) {}
}

impl ParseKeyed for NoExt {}
impl<T> ParseItem<T> for NoExt {}
impl ParseMethod for NoExt {}

/// Consumes the `= value` half of a `key = value` argument, or nothing for a bare flag, so a
/// [`ParseKeyed`] impl doesn't repeat the `=`-peek boilerplate.
#[inline]
pub fn eat_eq(input: ParseStream) -> syn::Result<()> {
    if input.peek(Token![=]) {
        input.parse::<Token![=]>()?;
    }

    Ok(())
}

/// A trailing comma between entries, tolerated and consumed.
pub fn eat_comma(input: ParseStream) -> bool {
    if input.peek(Token![,]) {
        let _ = input.parse::<Token![,]>();

        true
    } else {
        false
    }
}

/// Builds the "unknown argument `{key}`, expected one of: .." error, merging the base's common
/// keys with the extension's [`expected_keys`](ParseKeyed::expected_keys).
pub fn unknown_key_error<Ext: ParseKeyed>(key: &Ident, common: &[&str]) -> syn::Error {
    let mut keys: Vec<&str> = common.to_vec();
    keys.extend_from_slice(Ext::expected_keys());

    syn::Error::new(
        key.span(),
        format!(
            "unknown argument `{key}`, expected one of: {}",
            keys.join(", ")
        ),
    )
}
