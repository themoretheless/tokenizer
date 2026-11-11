//! Host adapters for JSON, JSONC, JSON5, and JSON Lines.

use std::borrow::Cow;

use crate::{
    LexToken, LexerOptions, Parse, ParseOptions, SemanticKind, SyntaxKind, TokenFlags, lex_with,
    parse_jsonl, parse_with, tokenize_with,
};
use themoretheless_tokenizer_core::{
    Capabilities, DialectDescriptor, DialectId, HostAnalysisOptions, HostDiagnostic, HostError,
    HostLanguage, HostSpan, HostToken, HostTokenization, LanguageDescriptor, LanguageId, Severity,
    Span, language_descriptor,
};

pub const LANGUAGE_ID: LanguageId = LanguageId::JSON;

// ─── JSON / JSONC ────────────────────────────────────────────────────────────

/// Host adapter for strict JSON and JSONC.
#[derive(Debug, Default, Clone, Copy)]
pub struct JsonHost;

static JSON_DIALECTS: &[DialectDescriptor] = &[
    DialectDescriptor {
        id: DialectId::STRICT,
        display_name: "Strict JSON",
        aliases: &[],
    },
    DialectDescriptor {
        id: DialectId::JSONC,
        display_name: "JSONC",
        aliases: &["json-with-comments"],
    },
];

pub static DESCRIPTOR: LanguageDescriptor = LanguageDescriptor {
    language: LANGUAGE_ID,
    display_name: "JSON",
    dialects: JSON_DIALECTS,
    default_dialect: DialectId::STRICT,
    aliases: &[],
    extensions: &[".json", ".jsonc"],
    mime_types: &["application/json", "application/jsonc"],
    capabilities: Capabilities::JSON_FULL,
    engine_version: env!("CARGO_PKG_VERSION"),
};

pub static ENGINE: JsonHost = JsonHost;

impl HostLanguage for JsonHost {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        let dialect = self.check_entry(source, opts)?;
        let options = match dialect {
            DialectId::JSONC => LexerOptions::jsonc(),
            _ => LexerOptions::strict(),
        };
        let result = lex_with(source, options);
        let mut tokens = Vec::with_capacity(result.tokens().len());
        for token in result.tokens() {
            tokens.push(HostToken {
                kind: Cow::Borrowed(syntax_kind_str(token.kind)),
                span: token.span.into(),
                error: token.has_error(),
            });
        }
        let diagnostics = result
            .diagnostics()
            .iter()
            .map(|diagnostic| HostDiagnostic {
                code: Cow::Borrowed(diagnostic.kind.code()),
                message: Cow::Owned(diagnostic.kind.to_string()),
                span: diagnostic.span.into(),
                severity: Severity::Error,
            })
            .collect();
        Ok(HostTokenization {
            tokens,
            diagnostics,
            valid: !result.has_errors(),
        })
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        let dialect = self.check_entry(source, opts)?;
        let options = match dialect {
            DialectId::JSONC => ParseOptions::jsonc(),
            _ => ParseOptions::strict(),
        };
        let result = tokenize_with(source, options);
        let mut tokens = Vec::with_capacity(result.tokens.len());
        for token in &result.tokens {
            tokens.push(HostToken {
                kind: Cow::Borrowed(semantic_kind_str(token.kind)),
                span: token.span.into(),
                error: matches!(token.kind, SemanticKind::Invalid),
            });
        }
        let diagnostics = result
            .diagnostics
            .iter()
            .map(|diagnostic| HostDiagnostic {
                code: Cow::Borrowed(diagnostic.kind.code()),
                message: Cow::Owned(diagnostic.kind.to_string()),
                span: diagnostic.span.into(),
                severity: Severity::Error,
            })
            .collect();
        Ok(HostTokenization {
            tokens,
            diagnostics,
            valid: result.is_valid(),
        })
    }
}

const fn syntax_kind_str(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::Whitespace => "whitespace",
        SyntaxKind::Bom => "bom",
        SyntaxKind::LineComment => "line-comment",
        SyntaxKind::BlockComment => "block-comment",
        SyntaxKind::LeftBrace => "left-brace",
        SyntaxKind::RightBrace => "right-brace",
        SyntaxKind::LeftBracket => "left-bracket",
        SyntaxKind::RightBracket => "right-bracket",
        SyntaxKind::Comma => "comma",
        SyntaxKind::Colon => "colon",
        SyntaxKind::String => "string",
        SyntaxKind::Identifier => "identifier",
        SyntaxKind::Number => "number",
        SyntaxKind::True => "true",
        SyntaxKind::False => "false",
        SyntaxKind::Null => "null",
        SyntaxKind::Error => "error",
    }
}

const fn semantic_kind_str(kind: SemanticKind) -> &'static str {
    match kind {
        SemanticKind::Property => "property",
        SemanticKind::String => "string",
        SemanticKind::Number => "number",
        SemanticKind::Boolean => "boolean",
        SemanticKind::Null => "null",
        SemanticKind::Punctuation => "punctuation",
        SemanticKind::Comment => "comment",
        SemanticKind::Whitespace => "whitespace",
        SemanticKind::Invalid => "invalid",
    }
}

// ─── Shared JSON5 / JSONL Helpers ────────────────────────────────────────────

#[inline]
fn shift(span: Span, offset: usize) -> Span {
    Span::new(span.start + offset, span.end + offset)
}

fn host_kind(
    source: &str,
    token: &LexToken,
    is_key: bool,
    is_trailing: bool,
    semantic: bool,
) -> &'static str {
    if token.kind == SyntaxKind::Comma && is_trailing {
        return "trailing-comma";
    }
    if is_key {
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
        SyntaxKind::Number => {
            let text = token.text(source).unwrap_or("");
            if text.ends_with("Infinity") {
                "infinity"
            } else if text.ends_with("NaN") {
                "nan"
            } else {
                "number"
            }
        }
        SyntaxKind::True => "true",
        SyntaxKind::False => "false",
        SyntaxKind::Null => "null",
        SyntaxKind::Error => "error",
    }
}

fn push_parse(parsed: &Parse<'_>, offset: usize, tokens: &mut Vec<HostToken>, semantic: bool) {
    let source = parsed.source();
    let keys = parsed.property_spans();
    let mut key_cursor = 0;
    let trailing = parsed.trailing_comma_spans();
    let mut trailing_cursor = 0;
    for token in parsed.lexed().tokens() {
        while keys
            .get(key_cursor)
            .is_some_and(|span| span.end <= token.span.start)
        {
            key_cursor += 1;
        }
        while trailing
            .get(trailing_cursor)
            .is_some_and(|span| span.end <= token.span.start)
        {
            trailing_cursor += 1;
        }
        let is_key = keys.get(key_cursor).copied() == Some(token.span);
        let is_trailing = trailing.get(trailing_cursor).copied() == Some(token.span);
        let error = token.has_error();
        tokens.push(HostToken {
            kind: Cow::Borrowed(if error {
                "error"
            } else {
                host_kind(source, token, is_key, is_trailing, semantic)
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

fn json5_tokenization(source: &str, semantic: bool) -> HostTokenization {
    let parsed = parse_with(source, ParseOptions::json5());
    let mut tokens = Vec::with_capacity(parsed.lexed().tokens().len());
    push_parse(&parsed, 0, &mut tokens, semantic);
    let mut diagnostics = Vec::with_capacity(parsed.diagnostics().len());
    push_diagnostics(&parsed, 0, &mut diagnostics);
    HostTokenization {
        tokens,
        diagnostics,
        valid: parsed.is_valid(),
    }
}

impl HostLanguage for Json5Host {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &JSON5_DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        let _ = self.check_entry(source, opts)?;
        Ok(json5_tokenization(source, false))
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        let _ = self.check_entry(source, opts)?;
        Ok(json5_tokenization(source, true))
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

fn jsonl_tokenization(source: &str, semantic: bool) -> HostTokenization {
    let document = parse_jsonl(source);
    let records = document.records();
    let breaks = document.line_breaks();
    let est_tokens: usize = records
        .iter()
        .map(|r| r.parsed().lexed().tokens().len())
        .sum::<usize>()
        + breaks.len();
    let mut tokens = Vec::with_capacity(est_tokens);
    let mut diagnostics = Vec::new();

    // Merge monotonic `records` and `line_breaks` in single linear order without sorting.
    let mut rec_idx = 0usize;
    let mut brk_idx = 0usize;
    while rec_idx < records.len() || brk_idx < breaks.len() {
        let take_break = match (records.get(rec_idx), breaks.get(brk_idx)) {
            (Some(rec), Some(brk)) => brk.start <= rec.span.start,
            (None, Some(_)) => true,
            (Some(_), None) => false,
            (None, None) => break,
        };
        if take_break {
            tokens.push(HostToken {
                kind: Cow::Borrowed("record-break"),
                span: HostSpan::from(breaks[brk_idx]),
                error: false,
            });
            brk_idx += 1;
        } else {
            let record = &records[rec_idx];
            push_parse(record.parsed(), record.span.start, &mut tokens, semantic);
            push_diagnostics(record.parsed(), record.span.start, &mut diagnostics);
            rec_idx += 1;
        }
    }

    HostTokenization {
        tokens,
        diagnostics,
        valid: document.is_valid(),
    }
}

impl HostLanguage for JsonlHost {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &JSONL_DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        let _ = self.check_entry(source, opts)?;
        Ok(jsonl_tokenization(source, false))
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        let _ = self.check_entry(source, opts)?;
        Ok(jsonl_tokenization(source, true))
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        Ok(self.lex(source, opts)?.diagnostics)
    }
}
