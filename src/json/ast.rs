//! Borrowing, order-preserving JSON value types.

use std::{borrow::Cow, error::Error, fmt};

use crate::Span;

/// The broad category of a parsed JSON value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ValueKind {
    Object,
    Array,
    String,
    Number,
    Boolean,
    Null,
}

/// A parsed JSON value borrowing raw strings and numbers from its source.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Value<'source> {
    Object(Object<'source>),
    Array(Array<'source>),
    String(StringValue<'source>),
    Number(Number<'source>),
    Boolean(Boolean),
    Null(Null),
}

impl<'source> Value<'source> {
    #[must_use]
    pub const fn kind(&self) -> ValueKind {
        match self {
            Self::Object(_) => ValueKind::Object,
            Self::Array(_) => ValueKind::Array,
            Self::String(_) => ValueKind::String,
            Self::Number(_) => ValueKind::Number,
            Self::Boolean(_) => ValueKind::Boolean,
            Self::Null(_) => ValueKind::Null,
        }
    }

    #[must_use]
    pub const fn span(&self) -> Span {
        match self {
            Self::Object(value) => value.span,
            Self::Array(value) => value.span,
            Self::String(value) => value.span,
            Self::Number(value) => value.span,
            Self::Boolean(value) => value.span,
            Self::Null(value) => value.span,
        }
    }

    #[must_use]
    pub const fn as_object(&self) -> Option<&Object<'source>> {
        if let Self::Object(value) = self {
            Some(value)
        } else {
            None
        }
    }

    #[must_use]
    pub const fn as_array(&self) -> Option<&Array<'source>> {
        if let Self::Array(value) = self {
            Some(value)
        } else {
            None
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        if let Self::String(value) = self {
            value.decoded()
        } else {
            None
        }
    }

    #[must_use]
    pub const fn as_number(&self) -> Option<&Number<'source>> {
        if let Self::Number(value) = self {
            Some(value)
        } else {
            None
        }
    }

    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        if let Self::Boolean(value) = self {
            Some(value.value)
        } else {
            None
        }
    }

    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null(_))
    }
}

/// An object that preserves member order and duplicate keys.
#[derive(Debug, Clone, PartialEq)]
pub struct Object<'source> {
    members: Vec<Member<'source>>,
    span: Span,
}

impl<'source> Object<'source> {
    pub(crate) fn new(members: Vec<Member<'source>>, span: Span) -> Self {
        Self { members, span }
    }

    #[must_use]
    pub fn members(&self) -> &[Member<'source>] {
        &self.members
    }

    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value<'source>> {
        self.members
            .iter()
            .find(|member| member.key.decoded() == Some(key))
            .map(|member| &member.value)
    }

    pub fn get_all<'object, 'key>(
        &'object self,
        key: &'key str,
    ) -> impl Iterator<Item = &'object Value<'source>> + use<'object, 'key, 'source> {
        self.members
            .iter()
            .filter(move |member| member.key.decoded() == Some(key))
            .map(|member| &member.value)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.members.len()
    }
}

/// One object property and value.
#[derive(Debug, Clone, PartialEq)]
pub struct Member<'source> {
    key: StringValue<'source>,
    value: Value<'source>,
    span: Span,
}

impl<'source> Member<'source> {
    pub(crate) fn new(key: StringValue<'source>, value: Value<'source>, span: Span) -> Self {
        Self { key, value, span }
    }

    #[must_use]
    pub const fn key(&self) -> &StringValue<'source> {
        &self.key
    }

    #[must_use]
    pub const fn value(&self) -> &Value<'source> {
        &self.value
    }

    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }
}

/// A JSON array.
#[derive(Debug, Clone, PartialEq)]
pub struct Array<'source> {
    elements: Vec<Value<'source>>,
    span: Span,
}

impl<'source> Array<'source> {
    pub(crate) fn new(elements: Vec<Value<'source>>, span: Span) -> Self {
        Self { elements, span }
    }

    #[must_use]
    pub fn elements(&self) -> &[Value<'source>] {
        &self.elements
    }

    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.elements.len()
    }
}

/// A raw and, when valid, decoded JSON string.
///
/// `raw` includes the surrounding quotes. `value` is `None` for malformed
/// strings, allowing recovery parses without presenting corrupt text as valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringValue<'source> {
    raw: &'source str,
    value: Option<Cow<'source, str>>,
    span: Span,
}

impl<'source> StringValue<'source> {
    pub(crate) fn new(raw: &'source str, value: Option<Cow<'source, str>>, span: Span) -> Self {
        Self { raw, value, span }
    }

    #[must_use]
    pub const fn raw(&self) -> &'source str {
        self.raw
    }

    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }

    #[must_use]
    pub fn decoded(&self) -> Option<&str> {
        self.value.as_deref()
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.value.is_some()
    }

    #[must_use]
    pub fn into_decoded(self) -> Option<Cow<'source, str>> {
        self.value
    }
}

/// A lossless JSON number. Conversion is explicit, so parsing never silently
/// rounds large integers through `f64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Number<'source> {
    raw: &'source str,
    span: Span,
    valid: bool,
}

impl<'source> Number<'source> {
    pub(crate) const fn new(raw: &'source str, span: Span, valid: bool) -> Self {
        Self { raw, span, valid }
    }

    #[must_use]
    pub const fn as_str(self) -> &'source str {
        self.raw
    }

    #[must_use]
    pub const fn span(self) -> Span {
        self.span
    }

    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.valid
    }

    pub fn as_i64(self) -> Result<i64, NumberError> {
        if !self.valid {
            return Err(NumberError::InvalidJsonNumber);
        }
        if self
            .raw
            .bytes()
            .any(|byte| matches!(byte, b'.' | b'e' | b'E'))
        {
            return Err(NumberError::NotInteger);
        }
        self.raw.parse().map_err(|_| NumberError::OutOfRange)
    }

    pub fn as_u64(self) -> Result<u64, NumberError> {
        if !self.valid {
            return Err(NumberError::InvalidJsonNumber);
        }
        if self.raw.starts_with('-') {
            return Err(NumberError::NegativeUnsigned);
        }
        if self
            .raw
            .bytes()
            .any(|byte| matches!(byte, b'.' | b'e' | b'E'))
        {
            return Err(NumberError::NotInteger);
        }
        self.raw.parse().map_err(|_| NumberError::OutOfRange)
    }

    pub fn as_f64(self) -> Result<f64, NumberError> {
        if !self.valid {
            return Err(NumberError::InvalidJsonNumber);
        }
        let value: f64 = self.raw.parse().map_err(|_| NumberError::InvalidFloat)?;
        if value.is_finite() {
            if value == 0.0 && self.raw.bytes().any(|byte| matches!(byte, b'1'..=b'9')) {
                Err(NumberError::Underflow)
            } else {
                Ok(value)
            }
        } else {
            Err(NumberError::NonFinite)
        }
    }
}

/// A number conversion failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NumberError {
    InvalidJsonNumber,
    NotInteger,
    NegativeUnsigned,
    OutOfRange,
    InvalidFloat,
    NonFinite,
    Underflow,
}

impl fmt::Display for NumberError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidJsonNumber => "the token is not a valid JSON number",
            Self::NotInteger => "the JSON number spelling is not an integer",
            Self::NegativeUnsigned => "a negative JSON number cannot be converted to u64",
            Self::OutOfRange => "the JSON integer is outside the requested type's range",
            Self::InvalidFloat => "the JSON number cannot be represented as an f64",
            Self::NonFinite => "the JSON number overflows to a non-finite f64",
            Self::Underflow => "the JSON number underflows to zero as an f64",
        })
    }
}

impl Error for NumberError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Boolean {
    value: bool,
    span: Span,
}

impl Boolean {
    pub(crate) const fn new(value: bool, span: Span) -> Self {
        Self { value, span }
    }

    #[must_use]
    pub const fn value(self) -> bool {
        self.value
    }

    #[must_use]
    pub const fn span(self) -> Span {
        self.span
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Null {
    span: Span,
}

impl Null {
    pub(crate) const fn new(span: Span) -> Self {
        Self { span }
    }

    #[must_use]
    pub const fn span(self) -> Span {
        self.span
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_conversions_are_explicit_and_checked() {
        let integer = Number {
            raw: "42",
            span: Span::new(0, 2),
            valid: true,
        };
        assert_eq!(integer.as_i64(), Ok(42));
        assert_eq!(integer.as_u64(), Ok(42));
        assert_eq!(integer.as_f64(), Ok(42.0));

        let huge = Number {
            raw: "1e9999",
            span: Span::new(0, 6),
            valid: true,
        };
        assert_eq!(huge.as_f64(), Err(NumberError::NonFinite));

        let tiny = Number {
            raw: "1e-9999",
            span: Span::new(0, 7),
            valid: true,
        };
        assert_eq!(tiny.as_f64(), Err(NumberError::Underflow));

        let decimal = Number {
            raw: "1.0",
            span: Span::new(0, 3),
            valid: true,
        };
        assert_eq!(decimal.as_i64(), Err(NumberError::NotInteger));
    }

    #[test]
    fn objects_preserve_and_expose_duplicate_keys() {
        let key = |raw| StringValue {
            raw,
            value: Some(Cow::Borrowed(&raw[1..raw.len() - 1])),
            span: Span::new(0, raw.len()),
        };
        let value = |value| {
            Value::Boolean(Boolean {
                value,
                span: Span::new(0, 1),
            })
        };
        let object = Object {
            members: vec![
                Member {
                    key: key("\"x\""),
                    value: value(true),
                    span: Span::new(0, 1),
                },
                Member {
                    key: key("\"x\""),
                    value: value(false),
                    span: Span::new(0, 1),
                },
            ],
            span: Span::new(0, 1),
        };
        assert_eq!(object.get_all("x").count(), 2);
        assert_eq!(object.get("x").and_then(Value::as_bool), Some(true));
    }
}
