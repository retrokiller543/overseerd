//! Stable namespaced identifier construction shared across framework layers.

use std::{error::Error, fmt};

const SEPARATOR: u8 = b'/';

/// Namespace reserved for framework-owned stable identities.
pub const FRAMEWORK_NAMESPACE: &str = "overseerd";

/// Runtime namespace comparisons shared by category-safe namespaced ID types.
///
/// Concrete ID types retain their inherent const namespace accessors because Rust does not yet
/// support const trait methods. This trait provides only the runtime abstraction needed for
/// cross-category namespace policy.
pub trait NamespacedIdType {
    /// Returns the identifier's owning namespace, prefer the const variant over this, this is mainly used for comparisons at runtime via trait delegation.
    #[doc(hidden)]
    fn ns(&self) -> &'static str;

    /// Returns whether this identifier belongs to `namespace`.
    #[inline(always)]
    fn is_in_namespace(&self, namespace: &str) -> bool {
        self.ns() == namespace
    }

    /// Returns whether this and another category-safe ID share a namespace.
    #[inline(always)]
    fn shares_namespace_with<T>(&self, other: &T) -> bool
    where
        T: NamespacedIdType + ?Sized,
    {
        self.ns() == other.ns()
    }
}

/// The reason a stable namespaced identifier is invalid.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum IdErrorKind {
    /// The identifier does not contain an owner and local name.
    MissingNamespace,
    /// A path segment starts with an unsupported byte.
    InvalidSegmentStart,
    /// A path segment contains an unsupported byte.
    InvalidCharacter,
}

impl fmt::Display for IdErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::MissingNamespace => {
                write!(
                    f,
                    "must contain at least two non-empty '{}' separated segments",
                    char::from(SEPARATOR)
                )?;

                return Ok(());
            }
            Self::InvalidSegmentStart => {
                "each segment must start with a lowercase ASCII letter or digit"
            }
            Self::InvalidCharacter => {
                "segments may contain only lowercase ASCII letters, digits, '.', '_', and '-'"
            }
        };

        f.write_str(message)
    }
}

impl Error for IdErrorKind {}

/// A validation failure for a stable namespaced identifier.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct InvalidNamespacedId {
    category: &'static str,
    value: &'static str,
    kind: IdErrorKind,
}

impl InvalidNamespacedId {
    /// Returns the category of identifier that rejected the value.
    pub const fn category(&self) -> &'static str {
        self.category
    }

    /// Returns the rejected identifier.
    pub const fn value(&self) -> &'static str {
        self.value
    }

    /// Returns why the identifier was rejected.
    pub const fn kind(&self) -> IdErrorKind {
        self.kind
    }

    /// Returns why the identifier was rejected.
    pub const fn reason(&self) -> IdErrorKind {
        self.kind
    }
}

impl fmt::Display for InvalidNamespacedId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid {} id '{}': {}",
            self.category, self.value, self.kind
        )
    }
}

impl Error for InvalidNamespacedId {}

/// Validated slices used by [`namespaced_id_type!`](crate::namespaced_id_type).
#[doc(hidden)]
pub struct NamespacedIdParts {
    pub value: &'static str,
    pub separator: usize,
}

/// Validates and splits a stable identifier at its first namespace separator.
#[doc(hidden)]
pub const fn parse_namespaced_id(
    category: &'static str,
    value: &'static str,
) -> Result<NamespacedIdParts, InvalidNamespacedId> {
    match validate(value) {
        Ok(()) => {
            let separator = separator_index(value);

            Ok(NamespacedIdParts { value, separator })
        }
        Err(kind) => Err(InvalidNamespacedId {
            category,
            value,
            kind,
        }),
    }
}

const fn separator_index(value: &'static str) -> usize {
    let bytes = value.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == SEPARATOR {
            return index;
        }

        index += 1;
    }

    value.len()
}

const fn validate(value: &'static str) -> Result<(), IdErrorKind> {
    let bytes = value.as_bytes();
    let mut index = 0;
    let mut segment_start = true;
    let mut separators = 0;

    while index < bytes.len() {
        let byte = bytes[index];

        if byte == SEPARATOR {
            if segment_start {
                return Err(IdErrorKind::MissingNamespace);
            }

            separators += 1;
            segment_start = true;
            index += 1;

            continue;
        }

        if segment_start {
            if !is_lowercase_or_digit(byte) {
                return Err(IdErrorKind::InvalidSegmentStart);
            }

            segment_start = false;
        } else if !is_segment_byte(byte) {
            return Err(IdErrorKind::InvalidCharacter);
        }

        index += 1;
    }

    if separators == 0 || segment_start {
        return Err(IdErrorKind::MissingNamespace);
    }

    Ok(())
}

const fn is_lowercase_or_digit(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit()
}

const fn is_segment_byte(byte: u8) -> bool {
    is_lowercase_or_digit(byte) || byte == b'.' || byte == b'_' || byte == b'-'
}

/// Defines a category-safe stable identifier backed by its value and separator position.
#[macro_export]
macro_rules! namespaced_id_type {
    ($(#[$meta:meta])* $visibility:vis struct $name:ident, $category:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        $visibility struct $name {
            value: &'static str,
            separator: usize,
        }

        impl $name {
            /// Validates and creates the stable identifier.
            pub const fn new(
                value: &'static str,
            ) -> ::core::result::Result<Self, $crate::id::InvalidNamespacedId> {
                match $crate::id::parse_namespaced_id($category, value) {
                    ::core::result::Result::Ok(parts) => ::core::result::Result::Ok(Self {
                        value: parts.value,
                        separator: parts.separator,
                    }),
                    ::core::result::Result::Err(error) => {
                        ::core::result::Result::Err(error)
                    }
                }
            }

            /// Returns the canonical identifier text.
            pub const fn as_str(&self) -> &'static str {
                self.value
            }

            /// Returns the identifier's owning namespace.
            pub const fn namespace(&self) -> &'static str {
                self.parts().0
            }

            /// Returns the identifier's name within its namespace.
            pub const fn name(&self) -> &'static str {
                self.local_path()
            }

            /// Returns the identifier's non-empty path within its namespace.
            pub const fn local_path(&self) -> &'static str {
                self.parts().1
            }

            /// Returns both parts of this id.
            pub const fn parts(&self) -> (&'static str, &'static str) {
                let (namespace, remainder) = self.value.split_at(self.separator);
                let (_, local_path) = remainder.split_at(1);

                (namespace, local_path)
            }
        }

        impl ::core::fmt::Display for $name {
            #[inline(always)]
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                formatter.write_str(self.value)
            }
        }

        impl ::core::convert::AsRef<str> for $name {
            #[inline(always)]
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl $crate::id::NamespacedIdType for $name {
            #[inline(always)]
            fn ns(&self) -> &'static str {
                self.namespace()
            }
        }
    };
}

/// Creates a validated stable identifier in const or runtime expressions.
#[macro_export]
macro_rules! namespaced_id {
    ($type:ty, $value:literal) => {{
        const ID: $type = match <$type>::new($value) {
            ::core::result::Result::Ok(id) => id,
            ::core::result::Result::Err(_) => {
                panic!(concat!("invalid namespaced identifier literal: ", $value))
            }
        };

        ID
    }};
}
