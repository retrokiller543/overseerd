use std::fmt;

use thiserror::Error;

/// The reason a stable composition identifier is invalid.
#[derive(Clone, Copy, Debug, Eq, Error, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum IdErrorKind {
    /// The identifier does not contain an owner and local name.
    #[error("must contain at least two non-empty '/'-separated segments")]
    MissingNamespace,
    /// A path segment starts with an unsupported byte.
    #[error("each segment must start with a lowercase ASCII letter or digit")]
    InvalidSegmentStart,
    /// A path segment contains an unsupported byte.
    #[error("segments may contain only lowercase ASCII letters, digits, '.', '_', and '-'")]
    InvalidCharacter,
}

/// A validation failure for a stable composition identifier.
#[derive(Clone, Copy, Debug, Eq, Error, Ord, PartialEq, PartialOrd)]
#[error("invalid composition id '{value}': {kind}")]
pub struct InvalidCompositionId {
    value: &'static str,
    kind: IdErrorKind,
}

impl InvalidCompositionId {
    /// Returns the rejected identifier.
    pub const fn value(&self) -> &'static str {
        self.value
    }

    /// Returns why the identifier was rejected.
    pub const fn kind(&self) -> IdErrorKind {
        self.kind
    }
}

macro_rules! composition_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(&'static str);

        impl $name {
            /// Validates and creates the stable identifier.
            pub const fn new(value: &'static str) -> Result<Self, InvalidCompositionId> {
                match validate(value) {
                    Ok(()) => Ok(Self(value)),
                    Err(kind) => Err(InvalidCompositionId { value, kind }),
                }
            }

            /// Returns the canonical identifier text.
            pub const fn as_str(self) -> &'static str {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.0)
            }
        }
    };
}

composition_id!(
    ProtocolId,
    "A stable namespaced protocol definition identity."
);
composition_id!(
    PluginId,
    "A stable namespaced plugin implementation identity."
);
composition_id!(
    PluginSlotId,
    "A stable namespaced capability slot that a plugin implementation may occupy."
);
composition_id!(
    ContributionId,
    "A stable contributor-local identity for one composition contribution."
);

const fn validate(value: &'static str) -> Result<(), IdErrorKind> {
    let bytes = value.as_bytes();
    let mut index = 0;
    let mut segment_start = true;
    let mut separators = 0;

    while index < bytes.len() {
        let byte = bytes[index];

        if byte == b'/' {
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
