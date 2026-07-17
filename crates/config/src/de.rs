use serde::de::value::StrDeserializer;
use serde::de::{
    self, DeserializeOwned, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess,
    SeqAccess, VariantAccess, Visitor,
};

use crate::error::{TemplateError, TemplateErrorKind};
use crate::resolve::{ResolveCtx, ResolvedDependency, ResolverChain, render_str};
use crate::value::{ConfigStr, ConfigValue, StrKind};

/// Deserializes `T` from a [`ConfigValue`] tree, resolving placeholders through the
/// resolver chain.
///
/// Pure and re-runnable: it performs no I/O beyond what the resolvers do and never
/// mutates the tree, so a hot-reload is simply "build a fresh tree and call this
/// again".
pub fn from_value<T: DeserializeOwned>(
    root: &ConfigValue,
    resolvers: &ResolverChain,
) -> Result<T, TemplateError> {
    from_value_in(root, root, resolvers)
}

/// Like [`from_value`], but deserializes `value` (typically a subtree) while
/// resolving placeholders against `root` (the full tree).
///
/// Property-path placeholders such as `${app.server.port}` are absolute paths from
/// the root, so deserializing the `app.server` subtree must still resolve them
/// against the whole config — not the subtree, where that path does not exist.
pub fn from_value_in<T: DeserializeOwned>(
    root: &ConfigValue,
    value: &ConfigValue,
    resolvers: &ResolverChain,
) -> Result<T, TemplateError> {
    from_value_in_with_dependencies(root, value, resolvers).map(|(value, _)| value)
}

/// Deserializes a value and returns the placeholder results observed by that exact
/// pass. Reload uses this to keep its committed dependency fingerprint synchronized
/// with the `T` it publishes, even for stateful resolvers.
pub(crate) fn from_value_in_with_dependencies<T: DeserializeOwned>(
    root: &ConfigValue,
    value: &ConfigValue,
    resolvers: &ResolverChain,
) -> Result<(T, Vec<ResolvedDependency>), TemplateError> {
    let mut ctx = ResolveCtx::new(root, resolvers);
    let de = ValueDeserializer {
        value,
        ctx: &mut ctx,
        path: String::new(),
    };

    let value = T::deserialize(de)?;

    Ok((value, ctx.into_resolved_dependencies()))
}

/// A serde `Deserializer` over a single `ConfigValue` node.
///
/// Scalar coercion is chosen by *which* `deserialize_*` method the target type calls:
/// a full `${...}` placeholder defers its type to that method, while a templated leaf
/// can only ever satisfy `deserialize_str`. The `path` is carried purely for error
/// context.
pub struct ValueDeserializer<'cfg, 'ctx, 'r> {
    value: &'cfg ConfigValue,
    ctx: &'ctx mut ResolveCtx<'cfg, 'r>,
    path: String,
}

impl<'cfg, 'ctx, 'r> ValueDeserializer<'cfg, 'ctx, 'r> {
    /// Stamps a failure with this node's path (or leaves it bare at the root).
    fn err(&self, kind: TemplateErrorKind) -> TemplateError {
        if self.path.is_empty() {
            TemplateError::Bare(kind)
        } else {
            TemplateError::at(self.path.clone(), kind)
        }
    }

    /// A `TypeMismatch` for this node against the `expected` label.
    fn mismatch(&self, expected: &'static str) -> TemplateError {
        self.err(TemplateErrorKind::TypeMismatch {
            expected,
            found: self.value.type_label(),
        })
    }

    /// The raw string a string leaf coerces from, plus its classification so callers
    /// can reject a partial placeholder in a non-string slot.
    ///
    /// A full placeholder resolves to its raw value (type chosen by the caller); a
    /// templated or literal leaf renders to one string.
    fn scalar_source(&mut self, s: &ConfigStr) -> Result<(StrKind, String), TemplateError> {
        match s.kind {
            StrKind::FullPlaceholder => {
                let placeholder = s.as_full().expect("full kind implies one placeholder");
                let raw = self.ctx.resolve_placeholder(placeholder)?;

                Ok((StrKind::FullPlaceholder, raw))
            }

            StrKind::Literal | StrKind::Templated => {
                let rendered = render_str(s, self.ctx)?;

                Ok((s.kind, rendered))
            }
        }
    }

    /// Resolves an integer node to `i128`, narrowing happens in the caller. A
    /// templated placeholder is rejected (it can only be a string).
    fn int_value(&mut self, target: &'static str) -> Result<i128, TemplateError> {
        match self.value {
            ConfigValue::Int(n) => Ok(*n),

            ConfigValue::Str(s) => {
                let (kind, raw) = self.scalar_source(s)?;

                if kind == StrKind::Templated {
                    return Err(self.err(TemplateErrorKind::PartialInNonString { target }));
                }

                raw.parse::<i128>()
                    .map_err(|_| self.err(TemplateErrorKind::ParseAs { target }))
            }

            _ => Err(self.mismatch(target)),
        }
    }

    /// Resolves a float node to `f64`. A templated placeholder is rejected.
    fn float_value(&mut self, target: &'static str) -> Result<f64, TemplateError> {
        match self.value {
            ConfigValue::Float(f) => Ok(*f),
            ConfigValue::Int(n) => Ok(*n as f64),

            ConfigValue::Str(s) => {
                let (kind, raw) = self.scalar_source(s)?;

                if kind == StrKind::Templated {
                    return Err(self.err(TemplateErrorKind::PartialInNonString { target }));
                }

                raw.parse::<f64>()
                    .map_err(|_| self.err(TemplateErrorKind::ParseAs { target }))
            }

            _ => Err(self.mismatch(target)),
        }
    }

    /// Builds the child path for a table key.
    fn child_key(&self, key: &str) -> String {
        if self.path.is_empty() {
            key.to_string()
        } else {
            format!("{}.{}", self.path, key)
        }
    }
}

/// Generates the eight fixed-width integer methods, which differ only in target type
/// and the visitor call.
macro_rules! deserialize_int {
    ($method:ident, $ty:ty, $visit:ident) => {
        fn $method<V: Visitor<'de>>(mut self, visitor: V) -> Result<V::Value, Self::Error> {
            let n = self.int_value(stringify!($ty))?;

            let narrowed = <$ty>::try_from(n).map_err(|_| {
                self.err(TemplateErrorKind::OutOfRange {
                    target: stringify!($ty),
                })
            })?;

            visitor.$visit(narrowed)
        }
    };
}

impl<'de, 'cfg, 'ctx, 'r> de::Deserializer<'de> for ValueDeserializer<'cfg, 'ctx, 'r> {
    type Error = TemplateError;

    fn deserialize_bool<V: Visitor<'de>>(mut self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            ConfigValue::Bool(b) => visitor.visit_bool(*b),

            ConfigValue::Str(s) => {
                let (kind, raw) = self.scalar_source(s)?;

                if kind == StrKind::Templated {
                    return Err(self.err(TemplateErrorKind::PartialInNonString { target: "bool" }));
                }

                let parsed = raw
                    .parse::<bool>()
                    .map_err(|_| self.err(TemplateErrorKind::ParseAs { target: "bool" }))?;

                visitor.visit_bool(parsed)
            }

            _ => Err(self.mismatch("bool")),
        }
    }

    deserialize_int!(deserialize_i8, i8, visit_i8);
    deserialize_int!(deserialize_i16, i16, visit_i16);
    deserialize_int!(deserialize_i32, i32, visit_i32);
    deserialize_int!(deserialize_i64, i64, visit_i64);
    deserialize_int!(deserialize_u8, u8, visit_u8);
    deserialize_int!(deserialize_u16, u16, visit_u16);
    deserialize_int!(deserialize_u32, u32, visit_u32);
    deserialize_int!(deserialize_u64, u64, visit_u64);

    fn deserialize_i128<V: Visitor<'de>>(mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let n = self.int_value("i128")?;

        visitor.visit_i128(n)
    }

    fn deserialize_u128<V: Visitor<'de>>(mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let n = self.int_value("u128")?;

        let narrowed = u128::try_from(n)
            .map_err(|_| self.err(TemplateErrorKind::OutOfRange { target: "u128" }))?;

        visitor.visit_u128(narrowed)
    }

    fn deserialize_f32<V: Visitor<'de>>(mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let f = self.float_value("f32")?;

        visitor.visit_f32(f as f32)
    }

    fn deserialize_f64<V: Visitor<'de>>(mut self, visitor: V) -> Result<V::Value, Self::Error> {
        let f = self.float_value("f64")?;

        visitor.visit_f64(f)
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let rendered = match self.value {
            ConfigValue::Str(s) => render_str(s, self.ctx)?,
            _ => return Err(self.mismatch("char")),
        };

        let mut chars = rendered.chars();
        let first = chars.next();

        match (first, chars.next()) {
            (Some(c), None) => visitor.visit_char(c),
            _ => Err(self.err(TemplateErrorKind::ParseAs { target: "char" })),
        }
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            ConfigValue::Str(s) => {
                let rendered = render_str(s, self.ctx)?;

                visitor.visit_string(rendered)
            }

            _ => Err(self.mismatch("string")),
        }
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.deserialize_str(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            ConfigValue::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            ConfigValue::Null => visitor.visit_unit(),
            _ => Err(self.mismatch("null")),
        }
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            ConfigValue::Array(items) => visitor.visit_seq(SeqWalk {
                items,
                index: 0,
                ctx: self.ctx,
                path: self.path,
            }),

            _ => Err(self.mismatch("array")),
        }
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            ConfigValue::Table(entries) => visitor.visit_map(MapWalk {
                entries,
                index: 0,
                ctx: self.ctx,
                path: self.path,
            }),

            _ => Err(self.mismatch("table")),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        match self.value {
            ConfigValue::Str(s) => {
                let variant = render_str(s, self.ctx)?;

                visitor.visit_enum(EnumWalk {
                    variant,
                    value: None,
                    ctx: self.ctx,
                    path: self.path,
                })
            }

            ConfigValue::Table(entries) if entries.len() == 1 => {
                let (key, value) = &entries[0];
                let child_path = self.child_key(key);

                visitor.visit_enum(EnumWalk {
                    variant: key.clone(),
                    value: Some(value),
                    ctx: self.ctx,
                    path: child_path,
                })
            }

            _ => Err(self.mismatch("enum")),
        }
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let _ = self.value;

        visitor.visit_unit()
    }

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            ConfigValue::Null => visitor.visit_unit(),
            ConfigValue::Bool(b) => visitor.visit_bool(*b),

            ConfigValue::Int(n) => match i64::try_from(*n) {
                Ok(small) => visitor.visit_i64(small),
                Err(_) => visitor.visit_i128(*n),
            },

            ConfigValue::Float(f) => visitor.visit_f64(*f),

            ConfigValue::Str(s) => {
                let rendered = render_str(s, self.ctx)?;

                visitor.visit_string(rendered)
            }

            ConfigValue::Array(_) => self.deserialize_seq(visitor),
            ConfigValue::Table(_) => self.deserialize_map(visitor),
        }
    }

    serde::forward_to_deserialize_any! { bytes byte_buf }
}

/// `SeqAccess` over an array node, handing each element a child deserializer.
struct SeqWalk<'cfg, 'ctx, 'r> {
    items: &'cfg [ConfigValue],
    index: usize,
    ctx: &'ctx mut ResolveCtx<'cfg, 'r>,
    path: String,
}

impl<'de, 'cfg, 'ctx, 'r> SeqAccess<'de> for SeqWalk<'cfg, 'ctx, 'r> {
    type Error = TemplateError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error> {
        if self.index >= self.items.len() {
            return Ok(None);
        }

        let value = &self.items[self.index];
        let child_path = if self.path.is_empty() {
            format!("[{}]", self.index)
        } else {
            format!("{}[{}]", self.path, self.index)
        };

        self.index += 1;

        let de = ValueDeserializer {
            value,
            ctx: &mut *self.ctx,
            path: child_path,
        };

        seed.deserialize(de).map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len() - self.index)
    }
}

/// `MapAccess` over a table node. Keys are literal strings; values get child
/// deserializers carrying the extended path.
struct MapWalk<'cfg, 'ctx, 'r> {
    entries: &'cfg [(String, ConfigValue)],
    index: usize,
    ctx: &'ctx mut ResolveCtx<'cfg, 'r>,
    path: String,
}

impl<'de, 'cfg, 'ctx, 'r> MapAccess<'de> for MapWalk<'cfg, 'ctx, 'r> {
    type Error = TemplateError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        if self.index >= self.entries.len() {
            return Ok(None);
        }

        let key: StrDeserializer<'_, TemplateError> =
            self.entries[self.index].0.as_str().into_deserializer();

        seed.deserialize(key).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        let (key, value) = &self.entries[self.index];
        let child_path = if self.path.is_empty() {
            key.clone()
        } else {
            format!("{}.{}", self.path, key)
        };

        self.index += 1;

        let de = ValueDeserializer {
            value,
            ctx: &mut *self.ctx,
            path: child_path,
        };

        seed.deserialize(de)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.entries.len() - self.index)
    }
}

/// `EnumAccess` for both representations: a bare string (unit variant) and a
/// single-entry table (tagged variant whose value drives the variant data).
struct EnumWalk<'cfg, 'ctx, 'r> {
    variant: String,
    value: Option<&'cfg ConfigValue>,
    ctx: &'ctx mut ResolveCtx<'cfg, 'r>,
    path: String,
}

impl<'de, 'cfg, 'ctx, 'r> EnumAccess<'de> for EnumWalk<'cfg, 'ctx, 'r> {
    type Error = TemplateError;
    type Variant = VariantWalk<'cfg, 'ctx, 'r>;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), Self::Error> {
        let variant_de: StrDeserializer<'_, TemplateError> =
            self.variant.as_str().into_deserializer();
        let variant = seed.deserialize(variant_de)?;
        let access = VariantWalk {
            value: self.value,
            ctx: self.ctx,
            path: self.path,
        };

        Ok((variant, access))
    }
}

/// The variant-data side of an enum: a unit variant carries no value, while
/// newtype/tuple/struct variants deserialize the table entry's value.
struct VariantWalk<'cfg, 'ctx, 'r> {
    value: Option<&'cfg ConfigValue>,
    ctx: &'ctx mut ResolveCtx<'cfg, 'r>,
    path: String,
}

impl<'de, 'cfg, 'ctx, 'r> VariantAccess<'de> for VariantWalk<'cfg, 'ctx, 'r> {
    type Error = TemplateError;

    fn unit_variant(self) -> Result<(), Self::Error> {
        match self.value {
            None | Some(ConfigValue::Null) => Ok(()),
            Some(other) => Err(TemplateError::at(
                self.path,
                TemplateErrorKind::TypeMismatch {
                    expected: "unit variant",
                    found: other.type_label(),
                },
            )),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, Self::Error> {
        let value = self.value.ok_or_else(|| {
            TemplateError::at(
                self.path.clone(),
                TemplateErrorKind::TypeMismatch {
                    expected: "newtype variant",
                    found: "unit",
                },
            )
        })?;

        seed.deserialize(ValueDeserializer {
            value,
            ctx: self.ctx,
            path: self.path,
        })
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        let value = self.value.ok_or_else(|| {
            TemplateError::at(
                self.path.clone(),
                TemplateErrorKind::TypeMismatch {
                    expected: "tuple variant",
                    found: "unit",
                },
            )
        })?;

        ValueDeserializer {
            value,
            ctx: self.ctx,
            path: self.path,
        }
        .deserialize_seq(visitor)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        let value = self.value.ok_or_else(|| {
            TemplateError::at(
                self.path.clone(),
                TemplateErrorKind::TypeMismatch {
                    expected: "struct variant",
                    found: "unit",
                },
            )
        })?;

        ValueDeserializer {
            value,
            ctx: self.ctx,
            path: self.path,
        }
        .deserialize_map(visitor)
    }
}
