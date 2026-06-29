//! The attribute-argument extension seam.
//!
//! Overseerd's attribute macros (`#[component]`, `#[service]`, a plugin's `#[controller]`, …)
//! share a common set of keys (`id`, `name`, `scope`, …) but each adds its own. Rather than
//! fork the parser per macro, the shared arg structs are **generic over an extension type**
//! that implements [`ParseKeyed`]: the core `Parse` impl handles the common keys and hands
//! any key it does not recognize to the extension. The extension receives *one already-read
//! key* and parses just that key's value — never the whole stream — so contributing a new
//! argument is a small, local, type-safe addition.

use proc_macro2::TokenStream;
use quote::ToTokens;
use syn::parse::ParseStream;
use syn::{Ident, Token};

/// One macro's set of extension attribute arguments.
///
/// Implementors recognize their own keys, parse each key's value, **and** emit their own
/// code contribution. The driving `Parse` impl (on the generic arg struct that embeds the
/// extension) reads the key ident, tries the common keys, and otherwise calls
/// [`parse_keyed`](Self::parse_keyed). At a framework-chosen splice site the extension's
/// [`ToTokens`] output is emitted into the expansion.
///
/// The [`ToTokens`] supertrait is what makes the seam *user-controlled control*: the
/// framework owns the generated skeleton (struct, factory, descriptor, slices) and the
/// plugin only fills the designated hole with its own tokens — it never rewrites the shape.
/// Use [`NoExt`] for a macro with no extension.
pub trait ParseKeyed: Default + ToTokens {
    /// Parse the value for `key` (the ident is already consumed) from `input`.
    ///
    /// - `Ok(true)` — `key` is recognized; its value (or, for a bare flag, nothing) has been
    ///   consumed from `input`.
    /// - `Ok(false)` — `key` is unknown; `input` is left untouched so the caller can error
    ///   with a merged "expected .." diagnostic.
    /// - `Err(..)` — `key` is recognized but its value is malformed (spanned at the value).
    ///
    /// A `key = value` argument first consumes the `=`; a bare-flag argument consumes
    /// nothing. Implementors handle both by peeking for [`Token![=]`].
    fn parse_keyed(&mut self, key: &Ident, input: ParseStream) -> syn::Result<bool>;

    /// The keys this extension recognizes, for the "unknown argument, expected .." message.
    fn expected_keys() -> &'static [&'static str] {
        &[]
    }
}

/// The no-extension marker: recognizes no keys and emits nothing, so an arg struct
/// `Args<NoExt>` accepts only its common keys and expands to only the framework skeleton.
#[derive(Default)]
pub struct NoExt;

impl ToTokens for NoExt {
    #[inline(always)]
    fn to_tokens(&self, _tokens: &mut TokenStream) {}
}

impl ParseKeyed for NoExt {
    #[inline(always)]
    fn parse_keyed(&mut self, _key: &Ident, _input: ParseStream) -> syn::Result<bool> {
        Ok(false)
    }
}

/// Consumes the `= value` half of a `key = value` argument, or nothing for a bare flag.
/// A small helper so [`ParseKeyed`] impls don't repeat the `=`-peek boilerplate.
#[inline]
pub fn eat_eq(input: ParseStream) -> syn::Result<()> {
    if input.peek(Token![=]) {
        input.parse::<Token![=]>()?;
    }

    Ok(())
}

/// Builds the "unknown argument `{key}`, expected one of: .." error, merging the common keys
/// with the extension's [`expected_keys`](ParseKeyed::expected_keys).
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

/// A trailing comma between `key[= value]` entries, tolerated and consumed.
pub fn eat_comma(input: ParseStream) -> bool {
    if input.peek(Token![,]) {
        let _ = input.parse::<Token![,]>();

        true
    } else {
        false
    }
}
