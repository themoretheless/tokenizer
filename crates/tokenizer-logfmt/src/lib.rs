//! A lossless logfmt engine: lexer, recovering record pass, pair-aware
//! semantic layer.
//!
//! logfmt is a whitespace-separated stream of `key=value` pairs, one record
//! per line. This engine owns its vocabulary — there is no `keyword`, no
//! `string`, no `number` kind, because a logfmt key is a `key`, a value is a
//! `bare-value` or a `quoted-value`, and the newline is a `record-break`.
//! Concatenating every token's text reconstructs the source byte-for-byte,
//! including for malformed input: bad spans are flagged, never dropped or
//! synthesized. No key name is special: `ts`, `level` and `err` lex exactly
//! like `a` or `trace-id`, because this is a format engine, not a log pipeline.
//!
//! ```
//! use themoretheless_tokenizer_logfmt::{SyntaxKind, parse, validate};
//!
//! let source = "ts=2026-09-24T10:00:00Z level=info msg=\"serving\" port=8080 dry-run\n";
//! let parsed = parse(source);
//! assert!(parsed.is_valid());
//! assert_eq!(parsed.records().len(), 1);
//! assert_eq!(parsed.pairs().len(), 5);
//! assert_eq!(parsed.lexed().joined(), source);
//!
//! let kinds = parsed.lexed().tokens().iter().map(|t| t.kind).collect::<Vec<_>>();
//! assert!(kinds.contains(&SyntaxKind::FlagKey));
//! assert!(kinds.contains(&SyntaxKind::RecordBreak));
//!
//! // Recovery keeps every byte and names the fault with a stable code.
//! let codes: Vec<&str> = validate("msg=\"oops").iter().map(|d| d.code).collect();
//! assert_eq!(codes, ["unterminated-value"]);
//! let stray: Vec<&str> = validate("=oops\n").iter().map(|d| d.code).collect();
//! assert_eq!(stray, ["missing-key"]);
//! let glued: Vec<&str> = validate("msg=\"x\"y\n").iter().map(|d| d.code).collect();
//! assert_eq!(glued, ["unexpected-token"]);
//! ```
//!
//! The semantic layer re-tags two things without moving a single span: the `=`
//! of a value-less pair (`msg=`) becomes `empty-value`, and a *bare* value
//! takes a `integer-value` / `float-value` / `boolean-value` / `null-value`
//! reading. A quoted `"007"` stays text and never lights up as a number.

#![forbid(unsafe_code)]

mod lexer;
mod parser;

pub use lexer::{LexToken, Lexed, SyntaxKind, TokenFlags, lex};

pub use parser::{DiagnosticKind, Pair, Parse, Record, Value, parse, validate};

pub use themoretheless_tokenizer_core::{Diagnostic, Span};

// ─── Host adapters ───────────────────────────────────────────────────────────

use std::borrow::Cow;

use themoretheless_tokenizer_core::{
    Capabilities, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostSpan,
    HostToken, HostTokenization, LanguageDescriptor, LanguageId, Severity, language_descriptor,
    require_default_dialect,
};

/// Real engine surface: no CST, cursor navigation or visitor API exists here.
const CAPABILITIES: Capabilities = Capabilities::LEX
    .union(Capabilities::PARSE)
    .union(Capabilities::SEMANTIC)
    .union(Capabilities::VALIDATE);

/// Host token kind: the record-format vocabulary, so an editor can tell a key
/// from a value, a quoted value from a bare one, and a record break from
/// whitespace.
fn lex_host_kind(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::Bom => "bom",
        SyntaxKind::Key => "key",
        SyntaxKind::Separator => "separator",
        SyntaxKind::BareValue => "bare-value",
        SyntaxKind::QuotedValue => "quoted-value",
        SyntaxKind::EscapedChar => "escaped-char",
        SyntaxKind::FlagKey => "flag-key",
        SyntaxKind::Whitespace => "whitespace",
        SyntaxKind::RecordBreak => "record-break",
        SyntaxKind::Error => "error",
    }
}

/// Semantic layer: the syntax kinds plus two readings that need the pair
/// structure, applied without moving a span.
///
/// * The `=` of a value-less pair is re-tagged `empty-value`. A value with no
///   bytes has no span of its own, and inventing a zero-width token would
///   break the lossless contract, so the separator carries the reading.
/// * A *bare* value is typed by what its bytes are. Quoted values are excluded
///   by construction, which is why a quoted `"42"` stays text.
fn semantic_host_kind(parsed: &Parse<'_>, token: LexToken) -> &'static str {
    if token.has_error() {
        return "error";
    }
    match token.kind {
        SyntaxKind::Separator if parsed.is_empty_value(token.span) => "empty-value",
        SyntaxKind::BareValue => {
            let text = token.text(parsed.lexed().source()).unwrap_or_default();
            match classify_bare_value(text) {
                Some(kind) => kind,
                None => "bare-value",
            }
        }
        kind => lex_host_kind(kind),
    }
}

/// What a bare value's bytes say they are. `None` keeps the token a plain
/// `bare-value`; empty text is not a value at all, so no zero-width reading is
/// ever invented. The reading is purely lexical — all-digits (with one optional
/// leading `-`) is an integer, so a zero-padded `007` keeps the integer reading
/// rather than being demoted to text by a rule the format does not state.
fn classify_bare_value(text: &str) -> Option<&'static str> {
    if text.eq_ignore_ascii_case("true") || text.eq_ignore_ascii_case("false") {
        return Some("boolean-value");
    }
    if text == "null" || text == "nil" {
        return Some("null-value");
    }
    if is_integer(text) {
        return Some("integer-value");
    }
    if is_float(text) {
        return Some("float-value");
    }
    None
}

/// `-?` followed by at least one digit and nothing else.
fn is_integer(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// Optional sign, digits with at least one `.` and/or an exponent — anything
/// numeric that [`is_integer`] did not already claim.
fn is_float(text: &str) -> bool {
    let body = text.strip_prefix(['-', '+']).unwrap_or(text);
    let (mantissa, exponent) = match body.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (body, None),
    };
    let mantissa_ok = if mantissa.contains('.') {
        let mut parts = mantissa.split('.');
        let head = parts.next().unwrap_or_default();
        let tail = parts.next().unwrap_or_default();
        parts.next().is_none()
            && (head.is_empty() || head.bytes().all(|byte| byte.is_ascii_digit()))
            && (tail.is_empty() || tail.bytes().all(|byte| byte.is_ascii_digit()))
            && !(head.is_empty() && tail.is_empty())
    } else {
        !mantissa.is_empty() && mantissa.bytes().all(|byte| byte.is_ascii_digit())
    };
    if !mantissa_ok || !(mantissa.contains('.') || exponent.is_some()) {
        return false; // a bare integer is not a float
    }
    let Some(exponent) = exponent else {
        return true;
    };
    let digits = exponent.strip_prefix(['-', '+']).unwrap_or(exponent);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn tokenization(parsed: &Parse<'_>, semantic: bool) -> HostTokenization {
    let tokens = parsed
        .lexed()
        .tokens()
        .iter()
        .map(|token| {
            let kind = if semantic {
                semantic_host_kind(parsed, *token)
            } else if token.has_error() {
                "error"
            } else {
                lex_host_kind(token.kind)
            };
            HostToken {
                kind: Cow::Borrowed(kind),
                span: HostSpan::from(token.span),
                error: token.has_error(),
            }
        })
        .collect();
    let diagnostics = parsed
        .diagnostics()
        .iter()
        .map(|diagnostic| HostDiagnostic {
            code: Cow::Borrowed(diagnostic.code),
            message: Cow::Borrowed(diagnostic.message),
            span: HostSpan::from(diagnostic.span),
            severity: Severity::Error,
        })
        .collect();
    HostTokenization {
        tokens,
        diagnostics,
        valid: parsed.is_valid(),
    }
}

fn analyze(
    descriptor: &LanguageDescriptor,
    source: &str,
    host: &HostAnalysisOptions,
    semantic: bool,
) -> Result<HostTokenization, HostError> {
    require_default_dialect(descriptor, host.dialect.as_ref())?;
    if host.limits.exceeds_input_bytes(source.len()) {
        return Err(HostError::InputTooLarge {
            max: host.limits.max_input_bytes,
            actual: source.len(),
        });
    }
    Ok(tokenization(&parse(source), semantic))
}

fn diagnose(
    descriptor: &LanguageDescriptor,
    source: &str,
    host: &HostAnalysisOptions,
) -> Result<Vec<HostDiagnostic>, HostError> {
    Ok(analyze(descriptor, source, host, true)?.diagnostics)
}

/// logfmt host adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

pub static ENGINE: Host = Host;

/// LEX, PARSE, SEMANTIC and VALIDATE are real; there is no node-identity
/// tree, cursor, or visitor here, so none of those capabilities are
/// advertised.
pub static DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::LOGFMT,
    "logfmt",
    &["logfmt"],
    &[".logfmt"],
    &["text/x-logfmt"],
    env!("CARGO_PKG_VERSION"),
    CAPABILITIES,
);

impl HostLanguage for Host {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        analyze(&DESCRIPTOR, source, opts, false)
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        analyze(&DESCRIPTOR, source, opts, true)
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        diagnose(&DESCRIPTOR, source, opts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every lex-layer kind the engine may emit.
    const LEX_VOCABULARY: [&str; 10] = [
        "bom",
        "key",
        "separator",
        "bare-value",
        "quoted-value",
        "escaped-char",
        "flag-key",
        "whitespace",
        "record-break",
        "error",
    ];

    /// The lex vocabulary plus the five readings the semantic layer may
    /// substitute; it never adds a kind with no span behind it.
    const SEMANTIC_VOCABULARY: [&str; 15] = [
        "bom",
        "key",
        "separator",
        "empty-value",
        "bare-value",
        "integer-value",
        "float-value",
        "boolean-value",
        "null-value",
        "quoted-value",
        "escaped-char",
        "flag-key",
        "whitespace",
        "record-break",
        "error",
    ];

    /// The eleven kinds a generated engine would emit for any format, kept
    /// verbatim from the repository's wiring gate.
    const GENERIC_KINDS: [&str; 11] = [
        "class",
        "comment",
        "function",
        "identifier",
        "keyword",
        "number",
        "punctuation",
        "string",
        "type",
        "variable",
        "whitespace",
    ];

    /// A document of the shape real logfmt writers produce. It must analyze
    /// clean: this is the gate the repository's format suite applies.
    const SAMPLE: &str = concat!(
        "ts=2026-09-24T10:00:00Z level=info msg=\"serving connections\" port=8080 dry-run\n",
        "trace-id=deadbeef retry=3 ratio=0.25 err=nil dropped=\n",
        "note=\"said \\\"hi\\\" and left\" user=ada😀\n",
    );

    fn distinct<'a>(kinds: &'a [&'a str]) -> Vec<&'a str> {
        let mut names: Vec<&str> = kinds.iter().map(|kind| kind.as_ref()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    fn host_lexed_joins(source: &str, tokenization: &HostTokenization) -> bool {
        let mut joined = String::new();
        for token in &tokenization.tokens {
            joined.push_str(&source[token.span.start..token.span.end]);
        }
        joined == source
    }

    #[test]
    fn host_lex_emits_only_the_logfmt_vocabulary() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.lex(SAMPLE, &opts).unwrap();
        assert!(tokenization.valid, "{:?}", tokenization.diagnostics);
        assert!(tokenization.diagnostics.is_empty());
        let names: Vec<&str> = tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect();
        for expected in [
            "key",
            "separator",
            "bare-value",
            "quoted-value",
            "escaped-char",
            "flag-key",
            "whitespace",
            "record-break",
        ] {
            assert!(names.contains(&expected), "missing {expected}: {names:?}");
        }
        for name in &names {
            assert!(LEX_VOCABULARY.contains(name), "off-vocabulary {name}");
        }
    }

    #[test]
    fn host_semantic_emits_only_the_logfmt_vocabulary() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.semantic_tokens(SAMPLE, &opts).unwrap();
        assert!(tokenization.valid);
        let collected: Vec<&str> = tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect();
        let names = distinct(&collected);
        for expected in [
            "integer-value",
            "float-value",
            "null-value",
            "empty-value",
            "quoted-value",
        ] {
            assert!(names.contains(&expected), "missing {expected}: {names:?}");
        }
        for name in &names {
            assert!(SEMANTIC_VOCABULARY.contains(name), "off-vocabulary {name}");
        }
        // The borrowed vocabularies a generated engine would emit stay absent.
        for banned in [
            "keyword",
            "string",
            "number",
            "identifier",
            "operator",
            "property",
        ] {
            assert!(!names.contains(&banned), "{banned} is not logfmt");
        }
    }

    #[test]
    fn both_layers_are_lossless_over_the_host_wire() {
        let opts = HostAnalysisOptions::default();
        for source in [SAMPLE, "msg=\"oops\n", "=x a=\"y\"z\n", "\r\n", ""] {
            for tokenization in [
                ENGINE.lex(source, &opts).unwrap(),
                ENGINE.semantic_tokens(source, &opts).unwrap(),
            ] {
                assert!(
                    host_lexed_joins(source, &tokenization),
                    "{source:?} did not round-trip"
                );
                for token in &tokenization.tokens {
                    assert!(
                        token.span.start < token.span.end,
                        "{source:?} carried a zero-width host span"
                    );
                }
            }
        }
    }

    #[test]
    fn host_lex_kinds_are_exact_for_a_two_item_record() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.lex("msg=hello dry-run\n", &opts).unwrap();
        let names: Vec<&str> = tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect();
        assert_eq!(
            names,
            vec![
                "key",
                "separator",
                "bare-value",
                "whitespace",
                "flag-key",
                "record-break",
            ]
        );
    }

    #[test]
    fn semantic_tokens_retag_without_moving_spans() {
        let opts = HostAnalysisOptions::default();
        let source = "a=1 b=2.5 c=true d=null e=\"007\" f=007 g= -x\n";
        let simple = ENGINE.semantic_tokens(source, &opts).unwrap();
        assert!(simple.valid, "{:?}", simple.diagnostics);
        let kinds: Vec<&str> = simple.tokens.iter().map(|t| t.kind.as_ref()).collect();
        assert_eq!(
            kinds,
            vec![
                "key",
                "separator",
                "integer-value",
                "whitespace",
                "key",
                "separator",
                "float-value",
                "whitespace",
                "key",
                "separator",
                "boolean-value",
                "whitespace",
                "key",
                "separator",
                "null-value",
                "whitespace",
                "key",
                "separator",
                "quoted-value",
                "whitespace",
                "key",
                "separator",
                "integer-value",
                "whitespace",
                "key",
                "empty-value",
                "whitespace",
                "flag-key",
                "record-break",
            ]
        );
        let syntax = ENGINE.lex(source, &opts).unwrap();
        assert_eq!(
            syntax
                .tokens
                .iter()
                .map(|token| token.span)
                .collect::<Vec<_>>(),
            simple
                .tokens
                .iter()
                .map(|token| token.span)
                .collect::<Vec<_>>(),
            "semantic layer must keep the syntax-layer spans"
        );
        // The syntax layer never claims the semantic readings.
        assert!(
            !syntax
                .tokens
                .iter()
                .any(|token| token.kind.as_ref() == "empty-value")
        );
    }

    #[test]
    fn quoted_values_never_take_a_value_reading() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE
            .semantic_tokens("a=\"007\" b=\"true\" c=\"1.5\" d=007\n", &opts)
            .unwrap();
        assert!(tokenization.valid);
        let kinds: Vec<&str> = tokenization
            .tokens
            .iter()
            .filter(|token| token.kind.ends_with("value"))
            .map(|token| token.kind.as_ref())
            .collect();
        assert_eq!(
            kinds,
            vec![
                "quoted-value",
                "quoted-value",
                "quoted-value",
                "integer-value"
            ],
            "only the bare value is typed"
        );
    }

    #[test]
    fn value_typing_ignores_the_key_entirely() {
        let opts = HostAnalysisOptions::default();
        for (key, value, expected) in [
            ("err", "null", "null-value"),
            ("whatever", "null", "null-value"),
            ("level", "true", "boolean-value"),
            ("x", "true", "boolean-value"),
            ("ts", "info", "bare-value"),
            ("duration", "12ms", "bare-value"),
        ] {
            let source = format!("{key}={value} j=1\n");
            let tokenization = ENGINE.semantic_tokens(&source, &opts).unwrap();
            assert!(tokenization.valid, "{source:?}");
            let kinds: Vec<&str> = tokenization
                .tokens
                .iter()
                .map(|token| token.kind.as_ref())
                .collect();
            assert_eq!(
                kinds,
                vec![
                    "key",
                    "separator",
                    expected,
                    "whitespace",
                    "key",
                    "separator",
                    "integer-value",
                    "record-break",
                ],
                "{source:?} typed by its value bytes alone"
            );
        }
    }

    #[test]
    fn value_classification_is_exhaustive() {
        for (text, expected) in [
            ("42", "integer-value"),
            ("-7", "integer-value"),
            ("+7", "bare-value"),
            ("0", "integer-value"),
            ("007", "integer-value"),
            ("-0", "integer-value"),
            ("4.2", "float-value"),
            ("-1.5e3", "float-value"),
            (".5", "float-value"),
            ("5.", "float-value"),
            ("2e10", "float-value"),
            ("1E5", "float-value"),
            ("e5", "bare-value"),
            ("1e", "bare-value"),
            ("1..2", "bare-value"),
            ("1:2", "bare-value"),
            ("12ms", "bare-value"),
            ("2026-09-24T10:00:00Z", "bare-value"),
            ("0.0.0.0:8080", "bare-value"),
            ("true", "boolean-value"),
            ("FALSE", "boolean-value"),
            ("TrUe", "boolean-value"),
            ("truex", "bare-value"),
            ("null", "null-value"),
            ("nil", "null-value"),
            ("NULL", "bare-value"),
            ("Null", "bare-value"),
            ("1=1", "bare-value"),
            ("-", "bare-value"),
            ("$0", "bare-value"),
        ] {
            let source = format!("k={text} j=1\n");
            let tokenization = ENGINE
                .semantic_tokens(&source, &HostAnalysisOptions::default())
                .unwrap();
            assert!(tokenization.valid, "{source:?}");
            let kind = tokenization
                .tokens
                .iter()
                .find(|token| token.span == HostSpan::from(Span::new(2, 2 + text.len())))
                .map(|token| token.kind.as_ref());
            assert_eq!(kind, Some(expected), "for {text:?}");
        }
    }

    #[test]
    fn bom_and_crlf_are_carried_by_both_layers() {
        let opts = HostAnalysisOptions::default();
        let source = "\u{FEFF}msg=hi\r\nkey=1\r\n";
        let tokenization = ENGINE.lex(source, &opts).unwrap();
        assert!(tokenization.valid, "{:?}", tokenization.diagnostics);
        let names: Vec<&str> = tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect();
        assert_eq!(names[0], "bom");
        assert_eq!(
            names.iter().filter(|kind| **kind == "record-break").count(),
            2
        );
        assert!(host_lexed_joins(source, &tokenization));
    }

    #[test]
    fn malformed_documents_surface_as_error_kinds_and_codes() {
        let opts = HostAnalysisOptions::default();
        let source = "a=1 msg=\"oops\"glued\n=x tail=\"unclosed\n";
        let tokenization = ENGINE.lex(source, &opts).unwrap();
        assert!(!tokenization.valid);
        assert!(
            tokenization
                .tokens
                .iter()
                .any(|token| token.kind.as_ref() == "error" && token.error),
            "{:?}",
            tokenization.tokens
        );
        let collected: Vec<&str> = tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect();
        let kinds = distinct(&collected);
        for name in &kinds {
            assert!(LEX_VOCABULARY.contains(name), "off-vocabulary {name}");
        }
    }

    #[test]
    fn host_diagnose_reports_only_logfmt_codes() {
        let opts = HostAnalysisOptions::default();
        let codes = |source: &str| -> Vec<String> {
            ENGINE
                .diagnose(source, &opts)
                .unwrap()
                .iter()
                .map(|diagnostic| diagnostic.code.to_string())
                .collect()
        };
        assert_eq!(codes("msg=\"oops"), vec!["unterminated-value".to_string()]);
        assert_eq!(codes("=oops\n"), vec!["missing-key".to_string()]);
        assert_eq!(codes("msg=\"x\"y\n"), vec!["unexpected-token".to_string()]);
        assert_eq!(codes("msg=x dry-run\n"), Vec::<String>::new());
        assert_eq!(codes(""), Vec::<String>::new());
    }

    #[test]
    fn descriptor_advertises_only_the_real_surface() {
        assert_eq!(DESCRIPTOR.capabilities, CAPABILITIES);
        assert!(DESCRIPTOR.capabilities.contains(
            Capabilities::LEX
                | Capabilities::PARSE
                | Capabilities::SEMANTIC
                | Capabilities::VALIDATE
        ));
        assert!(!DESCRIPTOR.capabilities.contains(Capabilities::CST));
        assert!(!DESCRIPTOR.capabilities.contains(Capabilities::NAVIGATE));
        assert!(!DESCRIPTOR.capabilities.contains(Capabilities::VISITOR));
        assert_eq!(DESCRIPTOR.engine_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(DESCRIPTOR.language, LanguageId::LOGFMT);
        assert_eq!(DESCRIPTOR.display_name, "logfmt");
        assert_eq!(DESCRIPTOR.extensions, [".logfmt"]);
        assert_eq!(DESCRIPTOR.mime_types, ["text/x-logfmt"]);
    }

    #[test]
    fn host_rejects_oversized_input() {
        let opts = HostAnalysisOptions::default().with_limits(
            themoretheless_tokenizer_core::InputLimits::conservative().max_input_bytes(4),
        );
        assert!(matches!(
            ENGINE.lex("msg=hello\n", &opts),
            Err(HostError::InputTooLarge { .. })
        ));
        assert!(matches!(
            ENGINE.semantic_tokens("msg=hello\n", &opts),
            Err(HostError::InputTooLarge { .. })
        ));
        assert!(matches!(
            ENGINE.diagnose("msg=hello\n", &opts),
            Err(HostError::InputTooLarge { .. })
        ));
        // At or below the budget the same calls succeed.
        assert!(ENGINE.lex("a=1\n", &opts).is_ok());
        assert!(ENGINE.lex("", &opts).is_ok());
    }

    /// The repository's format gate, encoded where it can be run: a
    /// representative valid document must analyze clean on both layers and must
    /// say something a generic lexer could not.
    #[test]
    fn representative_valid_document_is_diagnostic_free_on_both_layers() {
        let opts = HostAnalysisOptions::default();
        for (layer, tokenization) in [
            ("syntax", ENGINE.lex(SAMPLE, &opts).unwrap()),
            ("semantic", ENGINE.semantic_tokens(SAMPLE, &opts).unwrap()),
        ] {
            assert!(tokenization.valid, "{layer} rejected a valid document");
            assert!(tokenization.diagnostics.is_empty(), "{layer}");
            let specific: Vec<&str> = tokenization
                .tokens
                .iter()
                .map(|token| token.kind.as_ref())
                .filter(|kind| !GENERIC_KINDS.contains(kind))
                .collect();
            assert!(!specific.is_empty(), "{layer} emits only generic kinds");
            for expected in ["key", "separator", "record-break", "bare-value"] {
                assert!(specific.contains(&expected), "{layer}: {specific:?}");
            }
        }
        assert!(parse(SAMPLE).is_valid());
        assert!(validate(SAMPLE).is_empty());
    }

    #[test]
    fn empty_input_is_valid_and_carries_no_tokens() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.lex("", &opts).unwrap();
        assert!(tokenization.valid);
        assert!(tokenization.tokens.is_empty());
        assert!(tokenization.diagnostics.is_empty());
        let parsed = parse("");
        assert_eq!(parsed.records().len(), 0);
        assert_eq!(parsed.pairs().len(), 0);
    }
}
