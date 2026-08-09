//! Dependency-free tokenization primitives used by Peregon.
//!
//! Spans are UTF-8 byte offsets. This makes [`Token::text`] safe and keeps the
//! crate independent from a particular editor protocol. Browser adapters can
//! convert them to UTF-16 code-unit offsets at their boundary.
//!
//! Shared plugin primitives live in [`themoretheless_tokenizer_core`] and are
//! re-exported here. Language engines currently ship as modules of this facade
//! crate; they will move to separate crates per `docs/plugin-api-design.md`.

#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

#[cfg(feature = "json")]
pub use themoretheless_tokenizer_json as json;

#[cfg(feature = "url")]
pub use themoretheless_tokenizer_url as url;

#[cfg(feature = "xml")]
pub use themoretheless_tokenizer_xml as xml;
#[cfg(feature = "html")]
pub use themoretheless_tokenizer_html as html;
#[cfg(feature = "css")]
pub use themoretheless_tokenizer_css as css;
#[cfg(feature = "yaml")]
pub use themoretheless_tokenizer_yaml as yaml;
#[cfg(feature = "toml")]
pub use themoretheless_tokenizer_toml as toml;
#[cfg(feature = "markdown")]
pub use themoretheless_tokenizer_markdown as markdown;
#[cfg(feature = "sql")]
pub use themoretheless_tokenizer_sql as sql;
#[cfg(feature = "mongo")]
pub use themoretheless_tokenizer_mongo as mongo;
#[cfg(feature = "bash")]
pub use themoretheless_tokenizer_bash as bash;
#[cfg(feature = "powershell")]
pub use themoretheless_tokenizer_powershell as powershell;
#[cfg(feature = "javascript")]
pub use themoretheless_tokenizer_javascript as javascript;
#[cfg(feature = "typescript")]
pub use themoretheless_tokenizer_typescript as typescript;
#[cfg(feature = "python")]
pub use themoretheless_tokenizer_python as python;
#[cfg(feature = "java")]
pub use themoretheless_tokenizer_java as java;
#[cfg(feature = "csharp")]
pub use themoretheless_tokenizer_csharp as csharp;
#[cfg(feature = "go")]
pub use themoretheless_tokenizer_go as go;
#[cfg(feature = "php")]
pub use themoretheless_tokenizer_php as php;
#[cfg(feature = "ruby")]
pub use themoretheless_tokenizer_ruby as ruby;
#[cfg(feature = "c")]
pub use themoretheless_tokenizer_c as c;
#[cfg(feature = "cpp")]
pub use themoretheless_tokenizer_cpp as cpp;
#[cfg(feature = "rust")]
pub use themoretheless_tokenizer_rust as rust;
#[cfg(feature = "kotlin")]
pub use themoretheless_tokenizer_kotlin as kotlin;
#[cfg(feature = "swift")]
pub use themoretheless_tokenizer_swift as swift;
#[cfg(feature = "dart")]
pub use themoretheless_tokenizer_dart as dart;
#[cfg(feature = "r")]
pub use themoretheless_tokenizer_r as r;
#[cfg(feature = "visualbasic")]
pub use themoretheless_tokenizer_visualbasic as visualbasic;
#[cfg(feature = "fortran")]
pub use themoretheless_tokenizer_fortran as fortran;
#[cfg(feature = "matlab")]
pub use themoretheless_tokenizer_matlab as matlab;
#[cfg(feature = "delphi")]
pub use themoretheless_tokenizer_delphi as delphi;
#[cfg(feature = "scala")]
pub use themoretheless_tokenizer_scala as scala;
#[cfg(feature = "lua")]
pub use themoretheless_tokenizer_lua as lua;
#[cfg(feature = "perl")]
pub use themoretheless_tokenizer_perl as perl;
#[cfg(feature = "objectivec")]
pub use themoretheless_tokenizer_objectivec as objectivec;
#[cfg(feature = "julia")]
pub use themoretheless_tokenizer_julia as julia;
#[cfg(feature = "assembly")]
pub use themoretheless_tokenizer_assembly as assembly;
#[cfg(feature = "groovy")]
pub use themoretheless_tokenizer_groovy as groovy;
#[cfg(feature = "haskell")]
pub use themoretheless_tokenizer_haskell as haskell;
#[cfg(feature = "elixir")]
pub use themoretheless_tokenizer_elixir as elixir;
#[cfg(feature = "erlang")]
pub use themoretheless_tokenizer_erlang as erlang;
#[cfg(feature = "clojure")]
pub use themoretheless_tokenizer_clojure as clojure;
#[cfg(feature = "fsharp")]
pub use themoretheless_tokenizer_fsharp as fsharp;
#[cfg(feature = "ocaml")]
pub use themoretheless_tokenizer_ocaml as ocaml;
#[cfg(feature = "lisp")]
pub use themoretheless_tokenizer_lisp as lisp;
#[cfg(feature = "scheme")]
pub use themoretheless_tokenizer_scheme as scheme;
#[cfg(feature = "solidity")]
pub use themoretheless_tokenizer_solidity as solidity;
#[cfg(feature = "zig")]
pub use themoretheless_tokenizer_zig as zig;
#[cfg(feature = "nim")]
pub use themoretheless_tokenizer_nim as nim;
#[cfg(feature = "dlang")]
pub use themoretheless_tokenizer_dlang as dlang;
#[cfg(feature = "cobol")]
pub use themoretheless_tokenizer_cobol as cobol;
#[cfg(feature = "ada")]
pub use themoretheless_tokenizer_ada as ada;
#[cfg(feature = "prolog")]
pub use themoretheless_tokenizer_prolog as prolog;
#[cfg(feature = "abap")]
pub use themoretheless_tokenizer_abap as abap;
#[cfg(feature = "vhdl")]
pub use themoretheless_tokenizer_vhdl as vhdl;
#[cfg(feature = "verilog")]
pub use themoretheless_tokenizer_verilog as verilog;
#[cfg(feature = "graphql")]
pub use themoretheless_tokenizer_graphql as graphql;

pub mod api;
pub mod plugins;

#[cfg(feature = "web-bridge")]
#[doc(hidden)]
pub mod web_bridge;

pub use api::{Analysis, Source};
pub use plugins::{analyze_host, builtin_registry, register_builtins};

// Plugin core (spans, diagnostics, registry, host facade).
pub use themoretheless_tokenizer_core as core;
pub use themoretheless_tokenizer_core::{
    Capabilities, Capability, CapabilityError, ColumnEncoding, Diagnostic, DiagnosticKind,
    DialectDescriptor, DialectId, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage,
    HostSpan, HostToken, HostTokenization, InputLimits, LanguageDescriptor, LanguageId,
    LanguageKey, LanguageRegistry, LimitExceeded, LineColumn, LineIndex, LosslessViolation,
    PositionError, RegisterError, RegistryBuilder, Severity, Span, TokenLayer,
    verify_lossless_spans,
};

#[cfg(feature = "url")]
pub use themoretheless_tokenizer_url::{
    UrlKind, UrlToken, UrlTokenization, tokenize as tokenize_url,
    tokenize_and_validate as tokenize_url_validated, validate as validate_url,
};

/// Compatibility alias for the previous `source` module path.
pub mod source {
    pub use themoretheless_tokenizer_core::{ColumnEncoding, LineColumn, LineIndex, PositionError};
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
