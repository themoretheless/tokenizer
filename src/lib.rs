//! Dependency-free tokenization primitives used by Peregon.
//!
//! Spans are UTF-8 byte offsets. This makes [`Token::text`] safe and keeps the
//! crate independent from a particular editor protocol. Browser adapters can
//! convert them to UTF-16 code-unit offsets at their boundary.

#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

use std::ops::Range;

pub mod json;
pub mod source;

#[cfg(feature = "web-bridge")]
#[doc(hidden)]
pub mod web_bridge;

pub use source::{ColumnEncoding, LineColumn, LineIndex, PositionError};

/// Half-open UTF-8 byte range in the source string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Returns this span as a standard half-open range.
    #[must_use]
    pub const fn range(self) -> Range<usize> {
        self.start..self.end
    }

    /// Whether the half-open span contains a byte offset.
    #[must_use]
    pub const fn contains(self, offset: usize) -> bool {
        self.start <= offset && offset < self.end
    }

    /// The smallest span covering both inputs.
    #[must_use]
    pub const fn cover(self, other: Self) -> Self {
        Self::new(
            if self.start < other.start {
                self.start
            } else {
                other.start
            },
            if self.end > other.end {
                self.end
            } else {
                other.end
            },
        )
    }

    /// Whether both endpoints can safely index the given UTF-8 source.
    #[must_use]
    pub fn is_valid_for(self, source: &str) -> bool {
        self.start <= self.end
            && self.end <= source.len()
            && source.is_char_boundary(self.start)
            && source.is_char_boundary(self.end)
    }

    /// Returns the covered source text when the span is valid.
    #[must_use]
    pub fn slice(self, source: &str) -> Option<&str> {
        source.get(self.range())
    }
}

impl From<Span> for Range<usize> {
    fn from(span: Span) -> Self {
        span.start..span.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Property,
    String,
    Number,
    Boolean,
    Null,
    Punctuation,
    Whitespace,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    /// Returns the source fragment covered by this token.
    #[must_use]
    pub fn text(self, source: &str) -> Option<&str> {
        source.get(self.span.start..self.span.end)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    pub span: Span,
    pub code: &'static str,
    pub message: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tokenization {
    pub tokens: Vec<Token>,
    pub diagnostics: Vec<Diagnostic>,
}

pub trait Tokenizer {
    fn tokenize(&self, source: &str) -> Tokenization;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct JsonTokenizer;

impl Tokenizer for JsonTokenizer {
    fn tokenize(&self, source: &str) -> Tokenization {
        tokenize_json(source)
    }
}

/// Tokenizes JSON and incomplete JSON editor input without allocating token text.
#[must_use]
pub fn tokenize_json(source: &str) -> Tokenization {
    let bytes = source.as_bytes();
    let mut result = Tokenization::default();
    let mut cursor = 0;

    while cursor < bytes.len() {
        let start = cursor;
        let byte = bytes[cursor];

        if is_json_whitespace(byte) {
            cursor += 1;
            while cursor < bytes.len() && is_json_whitespace(bytes[cursor]) {
                cursor += 1;
            }
            push_token(&mut result, TokenKind::Whitespace, start, cursor);
            continue;
        }

        if matches!(byte, b'{' | b'}' | b'[' | b']' | b',' | b':') {
            cursor += 1;
            push_token(&mut result, TokenKind::Punctuation, start, cursor);
            continue;
        }

        if byte == b'"' {
            let string = scan_string(source, cursor);
            cursor = string.end;
            if string.valid {
                let kind = if next_non_whitespace(bytes, cursor) == Some(b':') {
                    TokenKind::Property
                } else {
                    TokenKind::String
                };
                push_token(&mut result, kind, start, cursor);
            } else {
                push_invalid(&mut result, start, cursor, string.code, string.message);
            }
            continue;
        }

        let atom_end = scan_atom_end(source, cursor);
        if byte == b'-' || byte.is_ascii_digit() {
            let valid_end = scan_json_number(bytes, cursor);
            cursor = atom_end;
            if valid_end == Some(atom_end) {
                push_token(&mut result, TokenKind::Number, start, cursor);
            } else {
                push_invalid(
                    &mut result,
                    start,
                    cursor,
                    "invalid-number",
                    "Invalid JSON number",
                );
            }
            continue;
        }

        cursor = atom_end;
        let atom = &source[start..cursor];
        let kind = match atom {
            "true" | "false" => Some(TokenKind::Boolean),
            "null" => Some(TokenKind::Null),
            _ => None,
        };
        if let Some(kind) = kind {
            push_token(&mut result, kind, start, cursor);
        } else {
            push_invalid(
                &mut result,
                start,
                cursor,
                "unexpected-token",
                "Unexpected token in JSON",
            );
        }
    }

    result
}

#[derive(Debug, Clone, Copy)]
struct StringScan {
    end: usize,
    valid: bool,
    code: &'static str,
    message: &'static str,
}

fn scan_string(source: &str, start: usize) -> StringScan {
    let bytes = source.as_bytes();
    let mut cursor = start + 1;
    let mut valid = true;
    let mut code = "invalid-string";
    let mut message = "Invalid JSON string";

    while cursor < bytes.len() {
        match bytes[cursor] {
            b'"' => {
                return StringScan {
                    end: cursor + 1,
                    valid,
                    code,
                    message,
                };
            }
            b'\\' => {
                cursor += 1;
                if cursor >= bytes.len() {
                    return StringScan {
                        end: bytes.len(),
                        valid: false,
                        code: "unterminated-string",
                        message: "Unterminated JSON string",
                    };
                }
                match bytes[cursor] {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => cursor += 1,
                    b'u' => {
                        let digits_end = cursor.saturating_add(5);
                        if digits_end <= bytes.len()
                            && bytes[cursor + 1..digits_end]
                                .iter()
                                .all(u8::is_ascii_hexdigit)
                        {
                            cursor = digits_end;
                        } else {
                            valid = false;
                            cursor += 1;
                        }
                    }
                    _ => {
                        valid = false;
                        cursor += 1;
                    }
                }
            }
            0x00..=0x1f => {
                valid = false;
                code = "unescaped-control-character";
                message = "JSON strings cannot contain unescaped control characters";
                cursor += 1;
            }
            _ => cursor = next_char_boundary(source, cursor),
        }
    }

    StringScan {
        end: bytes.len(),
        valid: false,
        code: "unterminated-string",
        message: "Unterminated JSON string",
    }
}

fn scan_json_number(bytes: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start;
    if bytes.get(cursor) == Some(&b'-') {
        cursor += 1;
    }

    match bytes.get(cursor) {
        Some(b'0') => cursor += 1,
        Some(b'1'..=b'9') => {
            cursor += 1;
            while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
                cursor += 1;
            }
        }
        _ => return None,
    }

    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let fraction_start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == fraction_start {
            return None;
        }
    }

    if matches!(bytes.get(cursor), Some(b'e' | b'E')) {
        cursor += 1;
        if matches!(bytes.get(cursor), Some(b'+' | b'-')) {
            cursor += 1;
        }
        let exponent_start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == exponent_start {
            return None;
        }
    }

    Some(cursor)
}

fn scan_atom_end(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let mut cursor = start;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if is_json_whitespace(byte)
            || matches!(byte, b'{' | b'}' | b'[' | b']' | b',' | b':' | b'"')
        {
            break;
        }
        cursor = next_char_boundary(source, cursor);
    }
    // The caller only invokes this function on a non-delimiter.
    if cursor == start {
        next_char_boundary(source, start)
    } else {
        cursor
    }
}

fn next_char_boundary(source: &str, start: usize) -> usize {
    start + source[start..].chars().next().map_or(1, char::len_utf8)
}

fn next_non_whitespace(bytes: &[u8], mut cursor: usize) -> Option<u8> {
    while bytes
        .get(cursor)
        .is_some_and(|byte| is_json_whitespace(*byte))
    {
        cursor += 1;
    }
    bytes.get(cursor).copied()
}

const fn is_json_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn push_token(result: &mut Tokenization, kind: TokenKind, start: usize, end: usize) {
    result.tokens.push(Token {
        kind,
        span: Span::new(start, end),
    });
}

fn push_invalid(
    result: &mut Tokenization,
    start: usize,
    end: usize,
    code: &'static str,
    message: &'static str,
) {
    let span = Span::new(start, end);
    result.tokens.push(Token {
        kind: TokenKind::Invalid,
        span,
    });
    result.diagnostics.push(Diagnostic {
        span,
        code,
        message,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn significant(source: &str) -> Vec<(TokenKind, &str)> {
        tokenize_json(source)
            .tokens
            .into_iter()
            .filter(|token| !matches!(token.kind, TokenKind::Whitespace | TokenKind::Punctuation))
            .map(|token| (token.kind, token.text(source).unwrap()))
            .collect()
    }

    #[test]
    fn classifies_properties_and_primitive_values() {
        let source = r#"{"name":"Москва","state":1,"active":true,"note":null}"#;
        assert_eq!(
            significant(source),
            vec![
                (TokenKind::Property, "\"name\""),
                (TokenKind::String, "\"Москва\""),
                (TokenKind::Property, "\"state\""),
                (TokenKind::Number, "1"),
                (TokenKind::Property, "\"active\""),
                (TokenKind::Boolean, "true"),
                (TokenKind::Property, "\"note\""),
                (TokenKind::Null, "null"),
            ]
        );
    }

    #[test]
    fn covers_source_with_valid_utf8_byte_spans() {
        let source = "{\n  \"emoji\": \"😀\"\n}";
        let result = tokenize_json(source);
        let rebuilt: String = result
            .tokens
            .iter()
            .map(|token| token.text(source).unwrap())
            .collect();
        assert_eq!(rebuilt, source);
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn supports_escaped_strings_and_exact_json_numbers() {
        let source = r#"{"text":"a\"b","value":-1.25e+3}"#;
        let tokens = significant(source);
        assert!(tokens.contains(&(TokenKind::String, r#""a\"b""#)));
        assert!(tokens.contains(&(TokenKind::Number, "-1.25e+3")));
    }

    #[test]
    fn reports_incomplete_and_invalid_input_without_losing_text() {
        for (source, code) in [
            (r#"{"name":"Москва"#, "unterminated-string"),
            ("[01]", "invalid-number"),
            ("[trueish]", "unexpected-token"),
            (r#"["\q"]"#, "invalid-string"),
        ] {
            let result = tokenize_json(source);
            assert_eq!(result.diagnostics[0].code, code, "{source}");
            let rebuilt: String = result
                .tokens
                .iter()
                .map(|token| token.text(source).unwrap())
                .collect();
            assert_eq!(rebuilt, source);
        }
    }

    #[test]
    fn accepts_only_json_whitespace() {
        let result = tokenize_json("[1,\u{a0}2]");
        assert_eq!(result.diagnostics[0].code, "unexpected-token");
    }
}
