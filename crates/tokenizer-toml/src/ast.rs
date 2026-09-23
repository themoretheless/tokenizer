//! Borrowing, order-preserving TOML value and document types.

use std::{borrow::Cow, error::Error, fmt};

use crate::Span;

/// The broad category of a parsed TOML value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ValueKind {
    String,
    Integer,
    Float,
    Boolean,
    DateTime,
    Array,
    Table,
    ArrayOfTables,
}

/// A parsed TOML value borrowing raw text from its source.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Value<'source> {
    String(StringValue<'source>),
    Integer(Integer<'source>),
    Float(Float<'source>),
    Boolean(Boolean),
    DateTime(DateTime<'source>),
    Array(Array<'source>),
    Table(Table<'source>),
    ArrayOfTables(ArrayOfTables<'source>),
}

impl<'source> Value<'source> {
    #[must_use]
    pub const fn kind(&self) -> ValueKind {
        match self {
            Self::String(_) => ValueKind::String,
            Self::Integer(_) => ValueKind::Integer,
            Self::Float(_) => ValueKind::Float,
            Self::Boolean(_) => ValueKind::Boolean,
            Self::DateTime(_) => ValueKind::DateTime,
            Self::Array(_) => ValueKind::Array,
            Self::Table(_) => ValueKind::Table,
            Self::ArrayOfTables(_) => ValueKind::ArrayOfTables,
        }
    }

    #[must_use]
    pub const fn span(&self) -> Span {
        match self {
            Self::String(value) => value.span,
            Self::Integer(value) => value.span,
            Self::Float(value) => value.span,
            Self::Boolean(value) => value.span,
            Self::DateTime(value) => value.span,
            Self::Array(value) => value.span,
            Self::Table(value) => value.span,
            Self::ArrayOfTables(value) => value.span,
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
    pub const fn as_integer(&self) -> Option<&Integer<'source>> {
        if let Self::Integer(value) = self {
            Some(value)
        } else {
            None
        }
    }

    #[must_use]
    pub const fn as_float(&self) -> Option<&Float<'source>> {
        if let Self::Float(value) = self {
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
    pub const fn as_date_time(&self) -> Option<&DateTime<'source>> {
        if let Self::DateTime(value) = self {
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
    pub const fn as_table(&self) -> Option<&Table<'source>> {
        if let Self::Table(value) = self {
            Some(value)
        } else {
            None
        }
    }

    #[must_use]
    pub const fn as_array_of_tables(&self) -> Option<&ArrayOfTables<'source>> {
        if let Self::ArrayOfTables(value) = self {
            Some(value)
        } else {
            None
        }
    }
}

/// A parsed TOML document: the merged root table of every expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Document<'source> {
    root: Table<'source>,
    span: Span,
}

impl<'source> Document<'source> {
    pub(crate) const fn new(root: Table<'source>, span: Span) -> Self {
        Self { root, span }
    }

    #[must_use]
    pub const fn root(&self) -> &Table<'source> {
        &self.root
    }

    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }
}

/// The syntactic origin of a table in the merged document tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TableOrigin {
    /// The document's root table.
    Root,
    /// Created or extended by a `[table]` or `[[array]]` header.
    Header,
    /// Created by the intermediate segments of a dotted key.
    Dotted,
    /// Written as an inline `{ ... }` literal; sealed after definition.
    Inline,
}

/// A TOML table preserving entry definition order.
#[derive(Debug, Clone, PartialEq)]
pub struct Table<'source> {
    entries: Vec<Entry<'source>>,
    origin: TableOrigin,
    header: Option<Span>,
    span: Span,
}

impl<'source> Table<'source> {
    pub(crate) fn new(
        entries: Vec<Entry<'source>>,
        origin: TableOrigin,
        header: Option<Span>,
        span: Span,
    ) -> Self {
        Self {
            entries,
            origin,
            header,
            span,
        }
    }

    #[must_use]
    pub fn entries(&self) -> &[Entry<'source>] {
        &self.entries
    }

    #[must_use]
    pub const fn origin(&self) -> TableOrigin {
        self.origin
    }

    /// The span of the `[header]` that defined this table, when one exists.
    #[must_use]
    pub const fn header(&self) -> Option<Span> {
        self.header
    }

    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }

    /// Looks up a direct child by comparing the decoded spelling of each
    /// entry's final key segment.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value<'source>> {
        self.entries
            .iter()
            .find(|entry| {
                entry
                    .key
                    .segments
                    .last()
                    .and_then(|segment| segment.decoded())
                    == Some(key)
            })
            .map(|entry| &entry.value)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// An array of tables created by repeated `[[header]]` definitions.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayOfTables<'source> {
    elements: Vec<Table<'source>>,
    span: Span,
}

impl<'source> ArrayOfTables<'source> {
    pub(crate) fn new(elements: Vec<Table<'source>>, span: Span) -> Self {
        Self { elements, span }
    }

    #[must_use]
    pub fn elements(&self) -> &[Table<'source>] {
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

/// One key/value pair as written, including dotted keys.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry<'source> {
    key: Key<'source>,
    value: Value<'source>,
    span: Span,
}

impl<'source> Entry<'source> {
    pub(crate) fn new(key: Key<'source>, value: Value<'source>, span: Span) -> Self {
        Self { key, value, span }
    }

    #[must_use]
    pub const fn key(&self) -> &Key<'source> {
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

/// A dotted key with the exact segments as written in the source.
#[derive(Debug, Clone, PartialEq)]
pub struct Key<'source> {
    segments: Vec<KeySegment<'source>>,
    span: Span,
}

impl<'source> Key<'source> {
    pub(crate) fn new(segments: Vec<KeySegment<'source>>, span: Span) -> Self {
        Self { segments, span }
    }

    #[must_use]
    pub fn segments(&self) -> &[KeySegment<'source>] {
        &self.segments
    }

    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }
}

/// One segment of a dotted key. `decoded` is `None` for quoted keys whose
/// escapes could not be decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct KeySegment<'source> {
    raw: &'source str,
    decoded: Option<Cow<'source, str>>,
    span: Span,
}

impl<'source> KeySegment<'source> {
    pub(crate) fn new(raw: &'source str, decoded: Option<Cow<'source, str>>, span: Span) -> Self {
        Self { raw, decoded, span }
    }

    #[must_use]
    pub const fn raw(&self) -> &'source str {
        self.raw
    }

    #[must_use]
    pub fn decoded(&self) -> Option<&str> {
        self.decoded.as_deref()
    }

    /// The decoded text, falling back to the raw spelling.
    #[must_use]
    pub fn text(&self) -> &str {
        self.decoded.as_deref().unwrap_or(self.raw)
    }

    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.decoded.is_some()
    }

    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }
}

/// The syntactic form a TOML string was written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StringKind {
    Basic,
    Literal,
    MultiLineBasic,
    MultiLineLiteral,
}

/// A raw and, when valid, decoded TOML string. `raw` includes the delimiters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringValue<'source> {
    raw: &'source str,
    decoded: Option<Cow<'source, str>>,
    span: Span,
    kind: StringKind,
}

impl<'source> StringValue<'source> {
    pub(crate) fn new(
        raw: &'source str,
        decoded: Option<Cow<'source, str>>,
        span: Span,
        kind: StringKind,
    ) -> Self {
        Self {
            raw,
            decoded,
            span,
            kind,
        }
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
    pub const fn kind(&self) -> StringKind {
        self.kind
    }

    #[must_use]
    pub fn decoded(&self) -> Option<&str> {
        self.decoded.as_deref()
    }

    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.decoded.is_some()
    }

    #[must_use]
    pub fn into_decoded(self) -> Option<Cow<'source, str>> {
        self.decoded
    }
}

/// A lossless TOML integer in any supported radix. Conversion is explicit, so
/// parsing never silently rounds values that exceed the requested type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Integer<'source> {
    raw: &'source str,
    span: Span,
    valid: bool,
}

impl<'source> Integer<'source> {
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
            return Err(NumberError::InvalidTomlNumber);
        }
        let cleaned = without_underscores(self.raw);
        if let Some((radix, digits)) = radix_of(&cleaned) {
            let magnitude =
                u64::from_str_radix(digits, radix).map_err(|_| NumberError::OutOfRange)?;
            return i64::try_from(magnitude).map_err(|_| NumberError::OutOfRange);
        }
        cleaned.parse().map_err(|_| NumberError::OutOfRange)
    }

    pub fn as_u64(self) -> Result<u64, NumberError> {
        if !self.valid {
            return Err(NumberError::InvalidTomlNumber);
        }
        let cleaned = without_underscores(self.raw);
        if cleaned.starts_with('-') {
            return Err(NumberError::NegativeUnsigned);
        }
        if let Some((radix, digits)) = radix_of(&cleaned) {
            return u64::from_str_radix(digits, radix).map_err(|_| NumberError::OutOfRange);
        }
        cleaned.parse().map_err(|_| NumberError::OutOfRange)
    }

    pub fn as_f64(self) -> Result<f64, NumberError> {
        if !self.valid {
            return Err(NumberError::InvalidTomlNumber);
        }
        let cleaned = without_underscores(self.raw);
        if let Some((radix, digits)) = radix_of(&cleaned) {
            let magnitude =
                u128::from_str_radix(digits, radix).map_err(|_| NumberError::OutOfRange)?;
            return Ok(magnitude as f64);
        }
        let value: f64 = cleaned.parse().map_err(|_| NumberError::InvalidFloat)?;
        if value.is_finite() {
            Ok(value)
        } else {
            Err(NumberError::NonFinite)
        }
    }
}

/// A lossless TOML float, including `inf` and `nan` spellings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Float<'source> {
    raw: &'source str,
    span: Span,
    valid: bool,
}

impl<'source> Float<'source> {
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

    pub fn as_f64(self) -> Result<f64, NumberError> {
        if !self.valid {
            return Err(NumberError::InvalidTomlNumber);
        }
        let cleaned = without_underscores(self.raw);
        let value = match cleaned.as_bytes() {
            b"inf" | b"+inf" => f64::INFINITY,
            b"-inf" => f64::NEG_INFINITY,
            b"nan" | b"+nan" | b"-nan" => f64::NAN,
            _ => cleaned.parse().map_err(|_| NumberError::InvalidFloat)?,
        };
        if value.is_finite() {
            Ok(value)
        } else if value.is_infinite() && cleaned.bytes().any(|byte| byte.is_ascii_digit()) {
            Err(NumberError::NonFinite)
        } else {
            Ok(value)
        }
    }
}

/// An integer or float conversion failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NumberError {
    InvalidTomlNumber,
    NegativeUnsigned,
    OutOfRange,
    InvalidFloat,
    NonFinite,
}

impl fmt::Display for NumberError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidTomlNumber => "the token is not a valid TOML number",
            Self::NegativeUnsigned => "a negative TOML integer cannot be converted to u64",
            Self::OutOfRange => "the TOML number is outside the requested type's range",
            Self::InvalidFloat => "the TOML float cannot be represented as an f64",
            Self::NonFinite => "the TOML number overflows to a non-finite f64",
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

/// The date-time shape a TOML token was written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DateTimeKind {
    OffsetDateTime,
    LocalDateTime,
    LocalDate,
    LocalTime,
}

/// A raw TOML date-time. No calendar conversion is performed; use `raw` with
/// a dedicated date-time library when arithmetic is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DateTime<'source> {
    raw: &'source str,
    span: Span,
    valid: bool,
    kind: DateTimeKind,
}

impl<'source> DateTime<'source> {
    pub(crate) const fn new(
        raw: &'source str,
        span: Span,
        valid: bool,
        kind: DateTimeKind,
    ) -> Self {
        Self {
            raw,
            span,
            valid,
            kind,
        }
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

    #[must_use]
    pub const fn kind(self) -> DateTimeKind {
        self.kind
    }
}

/// A TOML array of values.
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

fn without_underscores(raw: &str) -> Cow<'_, str> {
    if raw.contains('_') {
        Cow::Owned(raw.replace('_', ""))
    } else {
        Cow::Borrowed(raw)
    }
}

fn radix_of(cleaned: &str) -> Option<(u32, &str)> {
    if let Some(digits) = cleaned.strip_prefix("0x") {
        Some((16, digits))
    } else if let Some(digits) = cleaned.strip_prefix("0o") {
        Some((8, digits))
    } else if let Some(digits) = cleaned.strip_prefix("0b") {
        Some((2, digits))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_convert_in_every_radix() {
        let decimal = Integer {
            raw: "-1_000",
            span: Span::new(0, 6),
            valid: true,
        };
        assert_eq!(decimal.as_i64(), Ok(-1000));
        assert_eq!(decimal.as_u64(), Err(NumberError::NegativeUnsigned));
        assert_eq!(decimal.as_f64(), Ok(-1000.0));

        let hex = Integer {
            raw: "0xDE_AD",
            span: Span::new(0, 7),
            valid: true,
        };
        assert_eq!(hex.as_i64(), Ok(0xDEAD));
        assert_eq!(hex.as_u64(), Ok(0xDEAD));
        assert_eq!(hex.as_f64(), Ok(57_005.0));

        let binary = Integer {
            raw: "0b1010",
            span: Span::new(0, 6),
            valid: true,
        };
        assert_eq!(binary.as_i64(), Ok(10));

        let huge = Integer {
            raw: "0xFFFFFFFFFFFFFFFFF",
            span: Span::new(0, 19),
            valid: true,
        };
        assert_eq!(huge.as_i64(), Err(NumberError::OutOfRange));
        assert_eq!(huge.as_f64(), Ok(2.951_479_051_793_528_3e20));

        let invalid = Integer {
            raw: "01",
            span: Span::new(0, 2),
            valid: false,
        };
        assert_eq!(invalid.as_i64(), Err(NumberError::InvalidTomlNumber));
    }

    #[test]
    fn floats_map_special_spellings() {
        let inf = Float {
            raw: "-inf",
            span: Span::new(0, 4),
            valid: true,
        };
        assert_eq!(inf.as_f64(), Ok(f64::NEG_INFINITY));

        let nan = Float {
            raw: "+nan",
            span: Span::new(0, 4),
            valid: true,
        };
        assert!(nan.as_f64().is_ok_and(f64::is_nan));

        let invalid = Float {
            raw: "1e",
            span: Span::new(0, 2),
            valid: false,
        };
        assert_eq!(invalid.as_f64(), Err(NumberError::InvalidTomlNumber));
    }
}
