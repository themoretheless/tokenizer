//! [`HostLanguage`] adapter for the Forma UI / scene language.
//!
//! Bridges Forma's native UTF-16 frontend (`tokenize_borrowed`, `parse`,
//! `lint_document`) to the workspace's lossless UTF-8 byte-span [`HostLanguage`]
//! contract.

use std::borrow::Cow;

use themoretheless_tokenizer_core::{
    Capabilities, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostSpan,
    HostToken, HostTokenization, LanguageDescriptor, LanguageId, Severity, language_descriptor,
};

use crate::frontend::{BorrowedToken, Comment, lint_document, parse, tokenize_borrowed};

pub const LANGUAGE_ID: LanguageId = LanguageId::FORMA;

const CAPABILITIES: Capabilities = Capabilities::LEX
    .union(Capabilities::PARSE)
    .union(Capabilities::SEMANTIC)
    .union(Capabilities::VALIDATE);

#[derive(Debug, Default, Clone, Copy)]
pub struct FormaHost;

pub type Host = FormaHost;

pub static DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LANGUAGE_ID,
    "Forma",
    &["forma"],
    &[".forma"],
    &["text/x-forma"],
    env!("CARGO_PKG_VERSION"),
    CAPABILITIES,
);

pub static ENGINE: FormaHost = FormaHost;

const FORMA_KEYWORDS: &[&str] = &[
    "component",
    "design",
    "prop",
    "state",
    "let",
    "style",
    "tokens",
    "i18n",
    "slot",
    "if",
    "else",
    "for",
    "in",
    "empty",
    "match",
    "where",
    "shortcut",
    "enum",
    "struct",
    "import",
    "from",
    "as",
    "forward",
    "apply",
    "override",
    "true",
    "false",
    "null",
];

const FORMA_TYPES: &[&str] = &[
    "String", "Bool", "Boolean", "Number", "Int", "Float", "Color", "Length", "Duration", "Angle",
    "Asset", "List", "Object", "Map", "Record", "Any",
];

fn classify_syntax_token(text: &str) -> &'static str {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return "whitespace";
    }
    match bytes[0] {
        b'\'' | b'"' => "string",
        b'#' if bytes.len() > 1 && bytes[1..].iter().all(|b| b.is_ascii_hexdigit()) => "color",
        b'0'..=b'9' => "number",
        b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
            if matches!(text, "true" | "false") {
                "boolean"
            } else if text == "null" {
                "null"
            } else if FORMA_KEYWORDS.contains(&text) {
                "keyword"
            } else if FORMA_TYPES.contains(&text) {
                "type"
            } else {
                "identifier"
            }
        }
        b'{' | b'}' | b'[' | b']' | b'(' | b')' | b':' | b';' | b',' | b'@' | b'#' => "punctuation",
        _ => "operator",
    }
}

fn classify_semantic_token(
    tokens: &[BorrowedToken<'_>],
    idx: usize,
    base_kind: &'static str,
) -> &'static str {
    if base_kind != "identifier" {
        return base_kind;
    }
    let text = tokens[idx].text;
    let prev = idx.checked_sub(1).map(|p| tokens[p].text);
    let next = tokens.get(idx + 1).map(|n| n.text);

    match prev {
        Some("component" | "design" | "enum" | "struct") => return "class",
        Some("prop" | "state" | "let") => return "variable",
        Some("style") => return "function",
        _ => {}
    }
    if next == Some("(") {
        return "function";
    }
    if next == Some(":") {
        return "property";
    }
    if next == Some("{") && text.as_bytes().first().is_some_and(u8::is_ascii_uppercase) {
        return "class";
    }
    if text.starts_with("props.") || text.starts_with("state.") {
        return "variable";
    }
    if text.starts_with("tokens.") || text.starts_with("i18n.") {
        return "property";
    }
    base_kind
}

/// Convert a UTF-16 code-unit offset into a clamped UTF-8 byte offset.
fn utf16_to_byte_offset(source: &str, target_utf16: usize) -> usize {
    if source.is_ascii() {
        return target_utf16.min(source.len());
    }
    let mut u16_pos = 0usize;
    for (byte_idx, ch) in source.char_indices() {
        if u16_pos >= target_utf16 {
            return byte_idx;
        }
        u16_pos += ch.len_utf16();
    }
    source.len()
}

fn build_lossless_tokens(
    source: &str,
    tokens: &[BorrowedToken<'_>],
    comments: &[Comment<'_>],
    semantic: bool,
) -> Vec<HostToken> {
    let mut out = Vec::with_capacity(tokens.len() * 2 + comments.len() + 1);
    let mut cursor = 0usize;
    let mut ti = 0usize;
    let mut ci = 0usize;

    while ti < tokens.len() || ci < comments.len() {
        let take_comment = match (tokens.get(ti), comments.get(ci)) {
            (Some(t), Some(c)) => c.byte_start < t.byte_start,
            (None, Some(_)) => true,
            (Some(_), None) => false,
            (None, None) => break,
        };
        if take_comment {
            let c = &comments[ci];
            if c.byte_start > cursor {
                out.push(HostToken {
                    kind: Cow::Borrowed("whitespace"),
                    span: HostSpan {
                        start: cursor,
                        end: c.byte_start,
                    },
                    error: false,
                });
            }
            out.push(HostToken {
                kind: Cow::Borrowed("comment"),
                span: HostSpan {
                    start: c.byte_start,
                    end: c.byte_end,
                },
                error: false,
            });
            cursor = c.byte_end;
            ci += 1;
        } else {
            let t = &tokens[ti];
            if t.byte_start > cursor {
                out.push(HostToken {
                    kind: Cow::Borrowed("whitespace"),
                    span: HostSpan {
                        start: cursor,
                        end: t.byte_start,
                    },
                    error: false,
                });
            }
            let base_kind = classify_syntax_token(t.text);
            let kind = if semantic {
                classify_semantic_token(tokens, ti, base_kind)
            } else {
                base_kind
            };
            out.push(HostToken {
                kind: Cow::Borrowed(kind),
                span: HostSpan {
                    start: t.byte_start,
                    end: t.byte_end,
                },
                error: false,
            });
            cursor = t.byte_end;
            ti += 1;
        }
    }

    if cursor < source.len() {
        out.push(HostToken {
            kind: Cow::Borrowed("whitespace"),
            span: HostSpan {
                start: cursor,
                end: source.len(),
            },
            error: false,
        });
    }

    out
}

fn error_span_from_utf16(source: &str, utf16_offset: usize) -> HostSpan {
    if source.is_empty() {
        return HostSpan { start: 0, end: 0 };
    }
    let start = utf16_to_byte_offset(source, utf16_offset);
    if start < source.len() {
        let len = source[start..].chars().next().map_or(1, char::len_utf8);
        HostSpan {
            start,
            end: start + len,
        }
    } else {
        let prev = source.char_indices().next_back().map_or(0, |(idx, _)| idx);
        HostSpan {
            start: prev,
            end: source.len(),
        }
    }
}

fn collect_diagnostics(source: &str, semantic: bool) -> (Vec<HostDiagnostic>, bool) {
    if let Err(err) = parse(source) {
        return (
            vec![HostDiagnostic {
                code: Cow::Borrowed("syntax-error"),
                message: Cow::Owned(err.message),
                span: error_span_from_utf16(source, err.offset),
                severity: Severity::Error,
            }],
            false,
        );
    }

    if !semantic {
        return (Vec::new(), true);
    }

    let mut diagnostics = Vec::new();
    let mut valid = true;
    if let Ok(warnings) = lint_document(source)
        && let Some(items) = warnings.as_array()
    {
        for item in items {
            let code = item["code"].as_str().unwrap_or("lint-warning").to_owned();
            let message = item["message"].as_str().unwrap_or("").to_owned();
            let from_u16 = item["from"].as_u64().unwrap_or(0) as usize;
            let to_u16 = item["to"].as_u64().unwrap_or(from_u16 as u64) as usize;
            let span = if to_u16 > from_u16 {
                HostSpan {
                    start: utf16_to_byte_offset(source, from_u16),
                    end: utf16_to_byte_offset(source, to_u16),
                }
            } else {
                error_span_from_utf16(source, from_u16)
            };
            let is_type_err = code.starts_with("incompatible-") || code.starts_with("unknown-");
            if is_type_err {
                valid = false;
            }
            diagnostics.push(HostDiagnostic {
                code: Cow::Owned(code),
                message: Cow::Owned(message),
                span,
                severity: if is_type_err {
                    Severity::Error
                } else {
                    Severity::Warning
                },
            });
        }
    }
    (diagnostics, valid)
}

fn analyze_forma(
    source: &str,
    opts: &HostAnalysisOptions,
    semantic: bool,
) -> Result<HostTokenization, HostError> {
    let _ = ENGINE.check_entry(source, opts)?;
    match tokenize_borrowed(source) {
        Ok((tokens, comments, _)) => {
            let host_tokens = build_lossless_tokens(source, &tokens, &comments, semantic);
            let (diagnostics, valid) = collect_diagnostics(source, semantic);
            Ok(HostTokenization {
                tokens: host_tokens,
                diagnostics,
                valid,
            })
        }
        Err(err) => {
            let err_span = error_span_from_utf16(source, err.offset);
            let mut tokens = Vec::with_capacity(3);
            if err_span.start > 0 {
                tokens.push(HostToken {
                    kind: Cow::Borrowed("identifier"),
                    span: HostSpan {
                        start: 0,
                        end: err_span.start,
                    },
                    error: false,
                });
            }
            if err_span.end > err_span.start {
                tokens.push(HostToken {
                    kind: Cow::Borrowed("error"),
                    span: err_span,
                    error: true,
                });
            }
            if err_span.end < source.len() {
                tokens.push(HostToken {
                    kind: Cow::Borrowed("whitespace"),
                    span: HostSpan {
                        start: err_span.end,
                        end: source.len(),
                    },
                    error: false,
                });
            }
            Ok(HostTokenization {
                tokens,
                diagnostics: vec![HostDiagnostic {
                    code: Cow::Borrowed("syntax-error"),
                    message: Cow::Owned(err.message),
                    span: err_span,
                    severity: Severity::Error,
                }],
                valid: false,
            })
        }
    }
}

impl HostLanguage for FormaHost {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        analyze_forma(source, opts, false)
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        analyze_forma(source, opts, true)
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        Ok(analyze_forma(source, opts, true)?.diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use themoretheless_tokenizer_core::{InputLimits, Span, verify_lossless_spans};

    #[test]
    fn forma_host_is_lossless_and_enriches_semantics() {
        let source = "// card\ncomponent Card {\n  prop title: String = 'Привет';\n  Text { content: props.title; }\n}\n";
        let opts = HostAnalysisOptions::default();
        let lexed = ENGINE.lex(source, &opts).unwrap();
        assert!(lexed.valid, "{:?}", lexed.diagnostics);
        assert!(
            verify_lossless_spans(
                source,
                lexed
                    .tokens
                    .iter()
                    .map(|t| Span::new(t.span.start, t.span.end))
            )
            .is_ok()
        );
        let sem = ENGINE.semantic_tokens(source, &opts).unwrap();
        assert!(sem.tokens.iter().any(|t| t.kind == "class"));
        assert!(sem.tokens.iter().any(|t| t.kind == "variable"));
        assert!(sem.tokens.iter().any(|t| t.kind == "property"));
    }

    #[test]
    fn forma_host_enforces_input_limits() {
        let opts = HostAnalysisOptions::default()
            .with_limits(InputLimits::conservative().max_input_bytes(4));
        assert!(matches!(
            ENGINE.lex("component A {}", &opts),
            Err(HostError::InputTooLarge { .. })
        ));
    }
}
