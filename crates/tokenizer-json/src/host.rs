//! Host adapters for the two JSON-family formats that need their own identity.
//!
//! `json` itself is wired by the facade crate. JSON5 and JSONL get adapters
//! here because their vocabularies are this crate's lexer output filtered
//! through what each format actually allows: JSON5 adds bare-word keys,
//! single-quoted strings, hexadecimal and point-padded numbers, `Infinity`/
//! `NaN` and trailing commas; JSONL keeps strict JSON records and adds the
//! record break, which no JSON document has.

use std::borrow::Cow;

use crate::{LexToken, Parse, ParseOptions, SyntaxKind, TokenFlags, parse_jsonl, parse_with};
use themoretheless_tokenizer_core::{
    Capabilities, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostSpan,
    HostToken, HostTokenization, LanguageDescriptor, LanguageId, Severity, Span,
    language_descriptor, require_default_dialect,
};

fn shift(span: Span, offset: usize) -> Span {
    Span::new(span.start + offset, span.end + offset)
}

/// Host kind for one token of this crate's grammar.
///
/// `keys` and `trailing` come from the parse: whether a string is an object
/// key and whether a comma closes a container are structural facts, not
/// lexical ones. The syntax layer names each key by how it was written; the
/// semantic layer collapses them to `property`, as the json engine does.
fn host_kind(
    source: &str,
    token: &LexToken,
    keys: &[Span],
    trailing: &[Span],
    semantic: bool,
) -> &'static str {
    let text = token.text(source).unwrap_or("");
    if token.kind == SyntaxKind::Comma && trailing.contains(&token.span) {
        return "trailing-comma";
    }
    if keys.contains(&token.span) {
        if semantic {
            return "property";
        }
        return match token.kind {
            SyntaxKind::String if token.flags.contains(TokenFlags::SINGLE_QUOTED) => {
                "single-quoted-key"
            }
            SyntaxKind::String => "quoted-key",
            SyntaxKind::Identifier => "unquoted-key",
            SyntaxKind::Number => "numeric-key",
            _ => "keyword-key",
        };
    }
    match token.kind {
        SyntaxKind::Whitespace => "whitespace",
        SyntaxKind::Bom => "byte-order-mark",
        SyntaxKind::LineComment => "line-comment",
        SyntaxKind::BlockComment => "block-comment",
        SyntaxKind::LeftBrace => "left-brace",
        SyntaxKind::RightBrace => "right-brace",
        SyntaxKind::LeftBracket => "left-bracket",
        SyntaxKind::RightBracket => "right-bracket",
        SyntaxKind::Comma => "comma",
        SyntaxKind::Colon => "colon",
        SyntaxKind::Identifier => "identifier",
        SyntaxKind::String if token.flags.contains(TokenFlags::SINGLE_QUOTED) => {
            "single-quoted-string"
        }
        SyntaxKind::String => "string",
        SyntaxKind::Number if token.flags.contains(TokenFlags::HEX_NUMBER) => "hex-number",
        SyntaxKind::Number if text.ends_with("Infinity") => "infinity",
        SyntaxKind::Number if text.ends_with("NaN") => "nan",
        SyntaxKind::Number => "number",
        SyntaxKind::True => "true",
        SyntaxKind::False => "false",
        SyntaxKind::Null => "null",
        SyntaxKind::Error => "error",
    }
}

fn push_parse(parsed: &Parse<'_>, offset: usize, tokens: &mut Vec<HostToken>, semantic: bool) {
    let source = parsed.source();
    let keys = parsed.property_spans();
    let trailing = parsed.trailing_comma_spans();
    for token in parsed.lexed().tokens() {
        let error = token.has_error();
        tokens.push(HostToken {
            kind: Cow::Borrowed(if error {
                "error"
            } else {
                host_kind(source, token, keys, trailing, semantic)
            }),
            span: HostSpan::from(shift(token.span, offset)),
            error,
        });
    }
}

fn push_diagnostics(parsed: &Parse<'_>, offset: usize, out: &mut Vec<HostDiagnostic>) {
    for diagnostic in parsed.diagnostics() {
        out.push(HostDiagnostic {
            code: Cow::Borrowed(diagnostic.kind.code()),
            message: Cow::Owned(diagnostic.kind.to_string()),
            span: HostSpan::from(shift(diagnostic.span, offset)),
            severity: Severity::Error,
        });
    }
}

// ─── JSON5 ───────────────────────────────────────────────────────────────────

/// Host adapter for JSON5.
#[derive(Debug, Default, Clone, Copy)]
pub struct Json5Host;

pub static JSON5_ENGINE: Json5Host = Json5Host;

pub static JSON5_DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::JSON5,
    "JSON5",
    &[],
    &[".json5"],
    &["application/json5"],
    env!("CARGO_PKG_VERSION"),
    Capabilities::LEX
        .union(Capabilities::PARSE)
        .union(Capabilities::SEMANTIC)
        .union(Capabilities::VALIDATE),
);

fn json5_tokenization(source: &str, semantic: bool) -> Result<HostTokenization, HostError> {
    let parsed = parse_with(source, ParseOptions::json5());
    let mut tokens = Vec::with_capacity(parsed.lexed().tokens().len());
    push_parse(&parsed, 0, &mut tokens, semantic);
    let mut diagnostics = Vec::new();
    push_diagnostics(&parsed, 0, &mut diagnostics);
    Ok(HostTokenization {
        tokens,
        diagnostics,
        valid: parsed.is_valid(),
    })
}

impl HostLanguage for Json5Host {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &JSON5_DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        require_default_dialect(&JSON5_DESCRIPTOR, opts.dialect.as_ref())?;
        if opts.limits.exceeds_input_bytes(source.len()) {
            return Err(HostError::InputTooLarge {
                max: opts.limits.max_input_bytes,
                actual: source.len(),
            });
        }
        json5_tokenization(source, false)
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        require_default_dialect(&JSON5_DESCRIPTOR, opts.dialect.as_ref())?;
        if opts.limits.exceeds_input_bytes(source.len()) {
            return Err(HostError::InputTooLarge {
                max: opts.limits.max_input_bytes,
                actual: source.len(),
            });
        }
        json5_tokenization(source, true)
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        Ok(self.lex(source, opts)?.diagnostics)
    }
}

// ─── JSONL ───────────────────────────────────────────────────────────────────

/// Host adapter for JSON Lines.
#[derive(Debug, Default, Clone, Copy)]
pub struct JsonlHost;

pub static JSONL_ENGINE: JsonlHost = JsonlHost;

pub static JSONL_DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::JSONL,
    "JSON Lines",
    &["ndjson"],
    &[".jsonl", ".ndjson"],
    &["application/jsonl", "application/x-ndjson"],
    env!("CARGO_PKG_VERSION"),
    Capabilities::LEX
        .union(Capabilities::PARSE)
        .union(Capabilities::SEMANTIC)
        .union(Capabilities::VALIDATE),
);

fn jsonl_tokenization(source: &str, semantic: bool) -> Result<HostTokenization, HostError> {
    let document = parse_jsonl(source);
    let mut tokens = Vec::new();
    for record in document.records() {
        push_parse(record.parsed(), record.span.start, &mut tokens, semantic);
    }
    for line_break in document.line_breaks() {
        tokens.push(HostToken {
            kind: Cow::Borrowed("record-break"),
            span: HostSpan::from(*line_break),
            error: false,
        });
    }
    tokens.sort_by_key(|token| token.span.start);
    let mut diagnostics = Vec::new();
    for record in document.records() {
        push_diagnostics(record.parsed(), record.span.start, &mut diagnostics);
    }
    Ok(HostTokenization {
        tokens,
        diagnostics,
        valid: document.is_valid(),
    })
}

impl HostLanguage for JsonlHost {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &JSONL_DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        require_default_dialect(&JSONL_DESCRIPTOR, opts.dialect.as_ref())?;
        if opts.limits.exceeds_input_bytes(source.len()) {
            return Err(HostError::InputTooLarge {
                max: opts.limits.max_input_bytes,
                actual: source.len(),
            });
        }
        jsonl_tokenization(source, false)
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        require_default_dialect(&JSONL_DESCRIPTOR, opts.dialect.as_ref())?;
        if opts.limits.exceeds_input_bytes(source.len()) {
            return Err(HostError::InputTooLarge {
                max: opts.limits.max_input_bytes,
                actual: source.len(),
            });
        }
        jsonl_tokenization(source, true)
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        Ok(self.lex(source, opts)?.diagnostics)
    }
}
