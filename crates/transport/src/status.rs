//! The structured status code carried on RPC error responses, and its
//! framework-owned predefined category catalog. This is the on-the-wire
//! contract; the handler-facing ergonomics live in `crates/core`.

use bitflags::bitflags;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Bit offset of the flags section within the packed wire `u32`.
const FLAGS_SHIFT: u32 = 24;
/// Bit offset of the predefined section within the packed wire `u32`.
const PREDEFINED_SHIFT: u32 = 16;

bitflags! {
    /// Combinable control-flow flags carried in the flags section of a
    /// [`StatusCode`]. A consumer can branch on a single bit without
    /// deserializing the error body.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct Flags: u8 {
        /// The caller may safely retry the failed call.
        const RETRYABLE = 0b0000_0001;
    }
}

/// A structured status code carried on every error response.
///
/// In source it is three strictly-typed sections — combinable [`Flags`], a
/// framework-owned [`PredefinedCode`] category, and an application-owned custom
/// `u16` — so no section can ever be mistaken for or bleed into another. On the
/// wire it packs losslessly into a single `u32` (flags in bits 24–31, predefined
/// in bits 16–23, custom in bits 0–15); that packing lives entirely in the
/// `Serialize`/`Deserialize` impls and never surfaces in the API.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct StatusCode {
    flags: Flags,
    predefined: PredefinedCode,
    custom: u16,
}

impl StatusCode {
    /// Builds a status code from the framework-defined sections, with no custom
    /// value (the application section is left zero).
    pub const fn new(predefined: PredefinedCode, flags: Flags) -> Self {
        Self {
            flags,
            predefined,
            custom: 0,
        }
    }

    /// Builds a status code from the framework-defined sections plus an
    /// application-owned custom value.
    pub const fn new_with_custom(predefined: PredefinedCode, flags: Flags, custom: u16) -> Self {
        Self {
            flags,
            predefined,
            custom,
        }
    }

    /// The framework-owned predefined category.
    pub const fn predefined(self) -> PredefinedCode {
        self.predefined
    }

    /// The application-owned custom value. Opaque to the framework.
    pub const fn custom(self) -> u16 {
        self.custom
    }

    /// The combinable control-flow flags.
    pub const fn flags(self) -> Flags {
        self.flags
    }

    /// Tests whether every bit of `flag` is set.
    pub const fn contains(self, flag: Flags) -> bool {
        self.flags.contains(flag)
    }

    /// Packs the three sections into the wire `u32`.
    pub const fn raw(self) -> u32 {
        ((self.flags.bits() as u32) << FLAGS_SHIFT)
            | ((self.predefined.to_byte() as u32) << PREDEFINED_SHIFT)
            | self.custom as u32
    }

    /// Unpacks a wire `u32`. Total — any value decodes, with unknown flag bits
    /// retained for forward compatibility and unrecognized predefined bytes
    /// mapped to `UnknownErrorCode`.
    pub const fn from_raw(raw: u32) -> Self {
        Self {
            flags: Flags::from_bits_retain((raw >> FLAGS_SHIFT) as u8),
            predefined: PredefinedCode::from_byte((raw >> PREDEFINED_SHIFT) as u8),
            custom: raw as u16,
        }
    }
}

impl From<PredefinedCode> for StatusCode {
    fn from(predefined: PredefinedCode) -> Self {
        Self::new(predefined, Flags::empty())
    }
}

/// The packed `u32` is the wire form; the source form is the strict struct.
impl Serialize for StatusCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.raw().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for StatusCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = u32::deserialize(deserializer)?;

        Ok(Self::from_raw(raw))
    }
}

/// The framework-owned category occupying the predefined section of a
/// [`StatusCode`].
///
/// A closed catalog of framework categories, directly convertible to its wire
/// byte (`#[repr(u8)]`, so `code as u8`). Marked `#[non_exhaustive]` so new
/// categories can be added without breaking downstream matches; a byte this
/// version does not recognize decodes to `UnknownErrorCode` rather than failing,
/// keeping the wire decode total (FR-009).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[non_exhaustive]
#[repr(u8)]
pub enum PredefinedCode {
    /// Default for unset/unmapped codes.
    #[default]
    Empty = 0,

    /// Internal error.
    Internal = 1,
    /// Malformed or invalid request.
    BadInput = 2,
    /// No such route or resource.
    NotFound = 3,
    /// Caller not permitted.
    Unauthorized = 4,

    /// Sent Only if the predefined error code is unknown
    UnknownErrorCode = 255,
}

impl PredefinedCode {
    /// Decodes a predefined byte. Total: a byte with no known category maps to
    /// `UnknownErrorCode`, so it can never fail in this `const fn`.
    pub const fn from_byte(byte: u8) -> Self {
        match byte {
            0 => Self::Empty,
            1 => Self::Internal,
            2 => Self::BadInput,
            3 => Self::NotFound,
            4 => Self::Unauthorized,
            _ => Self::UnknownErrorCode,
        }
    }

    /// The underlying predefined byte.
    pub const fn to_byte(self) -> u8 {
        self as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_predefined_byte_decodes_totally() {
        // FR-009: decoding an unrecognized predefined byte is total, never a
        // parse error — an unknown category falls back to UnknownErrorCode.
        let code = StatusCode::from_raw(0x00FF_0000);

        assert_eq!(code.predefined(), PredefinedCode::UnknownErrorCode);
    }

    #[test]
    fn custom_section_is_isolated() {
        // FR-003: the custom value lives in its own section and cannot reach the
        // others, even at its maximum.
        let code = StatusCode::new_with_custom(PredefinedCode::NotFound, Flags::RETRYABLE, 0xABCD);

        assert_eq!(code.custom(), 0xABCD);
        assert_eq!(code.predefined(), PredefinedCode::NotFound);
        assert_eq!(code.flags(), Flags::RETRYABLE);

        let max = StatusCode::new_with_custom(PredefinedCode::NotFound, Flags::empty(), u16::MAX);

        assert_eq!(max.predefined(), PredefinedCode::NotFound);
        assert_eq!(max.custom(), u16::MAX);
    }

    #[test]
    fn flags_are_testable_and_isolated() {
        // FR-012: a flag is detectable on its own and leaves other sections alone.
        let code = StatusCode::new_with_custom(PredefinedCode::BadInput, Flags::RETRYABLE, 7);

        assert!(code.contains(Flags::RETRYABLE));
        assert_eq!(code.predefined(), PredefinedCode::BadInput);
        assert_eq!(code.custom(), 7);
    }

    #[test]
    fn round_trips_through_packed_wire_form() {
        let code =
            StatusCode::new_with_custom(PredefinedCode::Unauthorized, Flags::RETRYABLE, 0x1234);

        assert_eq!(StatusCode::from_raw(code.raw()), code);
    }
}
