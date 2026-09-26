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
    Capabilities, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostSpan,
    HostToken, HostTokenization, LanguageDescriptor, LanguageId, Severity, language_descriptor,
    require_default_dialect,
};

/// What this engine actually does, stated on its own: TOML has a hand-written
/// recovering grammar that rejects `[s` and `key =` and stays silent on valid
/// documents, so it advertises `VALIDATE` — which the shared fullkit parser
/// behind the wave languages cannot claim.
const CAPABILITIES: Capabilities = Capabilities::LEX
    .union(Capabilities::PARSE)
    .union(Capabilities::SEMANTIC)
    .union(Capabilities::VALIDATE);

/// Host token kind: the TOML spec's own lexical categories, so an editor (and
/// the playground) can tell a bare key from a quoted one, or an offset date
/// from an integer.
fn host_kind(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::Whitespace => "whitespace",
        SyntaxKind::Newline => "newline",
        SyntaxKind::Bom => "bom",
        SyntaxKind::LineComment => "comment",
        SyntaxKind::LeftBracket => "left-bracket",
        SyntaxKind::RightBracket => "right-bracket",
        SyntaxKind::LeftBrace => "left-brace",
        SyntaxKind::RightBrace => "right-brace",
        SyntaxKind::Equals => "equals",
        SyntaxKind::Comma => "comma",
        SyntaxKind::Dot => "dot",
        SyntaxKind::BareKey => "bare-key",
        SyntaxKind::BasicString => "basic-string",
        SyntaxKind::LiteralString => "literal-string",
        SyntaxKind::MultiLineBasicString => "multi-line-basic-string",
        SyntaxKind::MultiLineLiteralString => "multi-line-literal-string",
        SyntaxKind::Integer => "integer",
        SyntaxKind::Float => "float",
        SyntaxKind::HexInteger => "hex-integer",
        SyntaxKind::OctInteger => "octal-integer",
        SyntaxKind::BinInteger => "binary-integer",
        SyntaxKind::True => "true",
        SyntaxKind::False => "false",
        SyntaxKind::Inf => "inf",
        SyntaxKind::Nan => "nan",
        SyntaxKind::OffsetDateTime => "offset-date-time",
        SyntaxKind::LocalDateTime => "local-date-time",
        SyntaxKind::LocalDate => "local-date",
        SyntaxKind::LocalTime => "local-time",
        SyntaxKind::Error => "error",
    }
}

fn host_tokenization(parsed: &Parse<'_>) -> HostTokenization {
    let mut tokens = Vec::new();
    for token in parsed.lexed().tokens() {
        let error = token.has_error();
        tokens.push(HostToken {
            kind: Cow::Borrowed(if error {
                "error"
            } else {
                host_kind(token.kind)
            }),
            span: HostSpan::from(token.span),
            error,
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

pub static DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::TOML,
    "TOML",
    &[],
    &[".toml"],
    &["application/toml"],
    env!("CARGO_PKG_VERSION"),
    CAPABILITIES,
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
        assert!(kinds.contains(&"bare-key"), "{kinds:?}");
        assert!(kinds.contains(&"integer"), "{kinds:?}");
        assert!(kinds.contains(&"left-bracket"), "{kinds:?}");
        assert!(kinds.contains(&"newline"), "{kinds:?}");
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
