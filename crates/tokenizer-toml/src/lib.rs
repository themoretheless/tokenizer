//! A lossless TOML 1.0 lexer, recovering parser, and borrowing AST.
//!
//! Like the JSON engine, token text concatenation always reconstructs the
//! source byte-for-byte, so editors can tokenize incomplete documents. Spans
//! are UTF-8 byte offsets into the source string, and the AST borrows from the
//! parsed source instead of copying string data.
//!
//! ```
//! use themoretheless_tokenizer_toml::{SyntaxKind, parse, Value};
//!
//! let source = "[package]\nname = \"tokenizer\"\n";
//! let parsed = parse(source);
//! assert!(parsed.is_valid());
//! let package = parsed.document().root().get("package").and_then(Value::as_table);
//! assert!(package.is_some());
//!
//! let lexed = parsed.lexed();
//! let kinds = lexed.tokens().iter().map(|token| token.kind).collect::<Vec<_>>();
//! assert!(kinds.contains(&SyntaxKind::LeftBracket));
//! assert!(kinds.contains(&SyntaxKind::BareKey));
//! ```
//!
//! Newlines are significant [`SyntaxKind::Newline`] tokens because TOML uses
//! them to separate expressions. Grammatically invalid number and date-time
//! shapes still lex as single classified tokens (flagged
//! [`LexToken::has_error`]) so recovery can consume one broken value instead
//! of resynchronizing mid-atom. The parser applies TOML 1.0 redefinition rules
//! and reports violations as [`ParseDiagnostic`]s while preserving every
//! expression that follows.

mod ast;
mod lexer;
mod parser;

pub use ast::{
    Array, ArrayOfTables, Boolean, DateTime, DateTimeKind, Document, Entry, Float, Integer, Key,
    KeySegment, NumberError, StringKind, StringValue, Table, TableOrigin, Value, ValueKind,
};

pub use lexer::{
    DateTimeIssue, LexDiagnostic, LexDiagnosticKind, LexToken, Lexed, LexerOptions, NumberIssue,
    SyntaxKind, TokenFlags, lex, lex_with,
};

pub use parser::{
    MAX_SUPPORTED_DEPTH, Parse, ParseDiagnostic, ParseDiagnosticKind, ParseOptions, parse,
    parse_with,
};

pub use themoretheless_tokenizer_core::Span;

pub use themoretheless_tokenizer_core::{ColumnEncoding, LineIndex};

// ─── Host adapter ────────────────────────────────────────────────────────────

use std::borrow::Cow;

use themoretheless_tokenizer_core::{
    HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostSpan, HostToken,
    HostTokenization, LanguageDescriptor, LanguageId, Severity, SyntaxKind as CoreSyntaxKind,
    full_descriptor, require_default_dialect,
};

fn core_kind(kind: SyntaxKind) -> CoreSyntaxKind {
    match kind {
        SyntaxKind::Whitespace | SyntaxKind::Newline | SyntaxKind::Bom => {
            CoreSyntaxKind::Whitespace
        }
        SyntaxKind::LineComment => CoreSyntaxKind::LineComment,
        SyntaxKind::LeftBracket
        | SyntaxKind::RightBracket
        | SyntaxKind::LeftBrace
        | SyntaxKind::RightBrace
        | SyntaxKind::Equals
        | SyntaxKind::Comma
        | SyntaxKind::Dot => CoreSyntaxKind::Punctuation,
        SyntaxKind::BareKey => CoreSyntaxKind::Identifier,
        SyntaxKind::BasicString
        | SyntaxKind::LiteralString
        | SyntaxKind::MultiLineBasicString
        | SyntaxKind::MultiLineLiteralString => CoreSyntaxKind::StringLit,
        SyntaxKind::Integer
        | SyntaxKind::Float
        | SyntaxKind::HexInteger
        | SyntaxKind::OctInteger
        | SyntaxKind::BinInteger
        | SyntaxKind::OffsetDateTime
        | SyntaxKind::LocalDateTime
        | SyntaxKind::LocalDate
        | SyntaxKind::LocalTime => CoreSyntaxKind::NumberLit,
        SyntaxKind::True | SyntaxKind::False | SyntaxKind::Inf | SyntaxKind::Nan => {
            CoreSyntaxKind::Keyword
        }
        SyntaxKind::Error => CoreSyntaxKind::Error,
    }
}

fn host_tokenization(parsed: &Parse<'_>) -> HostTokenization {
    let mut tokens = Vec::new();
    for token in parsed.lexed().tokens() {
        let kind = if token.has_error() {
            CoreSyntaxKind::Error
        } else {
            core_kind(token.kind)
        };
        tokens.push(HostToken {
            kind: Cow::Borrowed(kind.as_str()),
            span: HostSpan::from(token.span),
            error: kind == CoreSyntaxKind::Error,
        });
    }
    let mut diagnostics = Vec::new();
    for diagnostic in parsed.diagnostics() {
        diagnostics.push(HostDiagnostic {
            code: Cow::Borrowed(diagnostic.kind.code()),
            message: Cow::Owned(diagnostic.kind.to_string()),
            span: HostSpan::from(diagnostic.span),
            severity: Severity::Error,
        });
    }
    let valid = parsed.is_valid();
    HostTokenization {
        tokens,
        diagnostics,
        valid,
    }
}

/// Host adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

pub static ENGINE: Host = Host;

pub static DESCRIPTOR: LanguageDescriptor = full_descriptor(
    LanguageId::TOML,
    "TOML",
    &[],
    &[".toml"],
    &["application/toml"],
    env!("CARGO_PKG_VERSION"),
);

impl HostLanguage for Host {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        if opts.limits.exceeds_input_bytes(source.len()) {
            return Err(HostError::InputTooLarge {
                max: opts.limits.max_input_bytes,
                actual: source.len(),
            });
        }
        Ok(host_tokenization(&parse(source)))
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        self.lex(source, opts)
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        require_default_dialect(&DESCRIPTOR, opts.dialect.as_ref())?;
        Ok(host_tokenization(&parse(source)).diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lossless_lex() {
        let source = "[package]\nname = 'tokenizer' # comment\n";
        assert!(
            themoretheless_tokenizer_core::verify_lossless_spans(
                source,
                lex(source).tokens().iter().map(|t| t.span)
            )
            .is_ok()
        );
    }

    #[test]
    fn parse_smoke() {
        let source = "title = \"tokenizer\"\ncount = 4\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        assert_eq!(
            parsed
                .document()
                .root()
                .get("count")
                .and_then(Value::as_integer)
                .map(|integer| integer.as_i64().unwrap()),
            Some(4)
        );
    }

    #[test]
    fn host_lex_uses_engine_tokens() {
        let source = "[table]\nkey = 1\n";
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.lex(source, &opts).unwrap();
        assert!(tokenization.valid);
        assert!(tokenization.diagnostics.is_empty());
        let kinds: Vec<&str> = tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect();
        assert!(kinds.contains(&"punctuation"));
        assert!(kinds.contains(&"number"));
    }

    #[test]
    fn host_diagnose_reports_duplicate_key() {
        let source = "a = 1\na = 2\n";
        let opts = HostAnalysisOptions::default();
        let diagnostics = ENGINE.diagnose(source, &opts).unwrap();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "duplicate-key")
        );
    }
}
