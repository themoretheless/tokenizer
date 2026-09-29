//! A lossless CSV and TSV engine: delimiter-parameterised lexer, recovering
//! record pass, and field-aware semantic layer.
//!
//! CSV here means RFC 4180 with its well-known leniencies (unquoted fields
//! may hold stray quotes; a quoted field may span CRLFs), and TSV means the
//! same grammar with the tab as delimiter and quoting *off*: a `"` is field
//! text and a comma is field text. The engine owns its vocabulary — there is
//! no `comment`, no `string`, no `whitespace` kind, because in this format a
//! space is field content and a newline is a `record-break`. Concatenating
//! every token's text reconstructs the source byte-for-byte, including for
//! malformed input: bad spans are flagged, never dropped or synthesized.
//!
//! ```
//! use themoretheless_tokenizer_csv::{Options, SyntaxKind, parse};
//!
//! let source = "name,score\nada,42\n\"grace, h.\",\"9\"\n";
//! let parsed = parse(source, Options::CSV);
//! assert!(parsed.is_valid());
//! assert_eq!(parsed.records().len(), 3);
//! assert_eq!(parsed.lexed().joined(), source);
//!
//! let kinds = parsed.lexed().tokens().iter().map(|t| t.kind).collect::<Vec<_>>();
//! assert!(kinds.contains(&SyntaxKind::Quote));
//! assert!(kinds.contains(&SyntaxKind::RecordBreak));
//!
//! // The empty middle field of ",," style rows carries no span at all.
//! let empty = parse("a,,b\n", Options::CSV);
//! assert_eq!(empty.fields()[1].span, None);
//! ```
//!
//! The semantic layer re-tags plain `field` tokens by what they hold
//! (`quoted-field`, `integer-field`, `decimal-field`, `boolean-field`) while
//! keeping every span identical, so a quoted `"007"` stays text and never
//! lights up as a number.

#![forbid(unsafe_code)]

mod lexer;
mod parser;

pub use lexer::{Delimiter, LexToken, Lexed, Options, SyntaxKind, TokenFlags, lex};

pub use parser::{DiagnosticKind, Field, Parse, Record, parse, validate};

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

/// Host token kind: the delimiter-format vocabulary, so an editor can tell a
/// quoted field from a bare one and a record break from punctuation.
fn lex_host_kind(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::Bom => "bom",
        SyntaxKind::HeaderField => "header-field",
        SyntaxKind::Field => "field",
        SyntaxKind::Quote => "quote",
        SyntaxKind::EscapedQuote => "escaped-quote",
        SyntaxKind::Delimiter => "delimiter",
        SyntaxKind::RecordBreak => "record-break",
        SyntaxKind::Error => "error",
    }
}

/// Semantic layer: only plain `field` tokens are re-tagged by what they hold.
/// `header-field`, `delimiter`, `quote`, `escaped-quote` and `record-break`
/// keep their syntax-layer kind, and the span never moves.
fn semantic_host_kind(token: LexToken, source: &str) -> &'static str {
    if token.has_error() {
        return "error";
    }
    if token.kind == SyntaxKind::Field {
        if token.is_quoted() {
            return "quoted-field";
        }
        let text = token.text(source).unwrap_or_default();
        return match classify_bare_field(text) {
            Some(kind) => kind,
            None => "field",
        };
    }
    lex_host_kind(token.kind)
}

/// What an unquoted field's bytes say they are. `None` keeps the token a
/// plain `field`; empty text is not a number, so no zero-width reading is
/// ever invented.
fn classify_bare_field(text: &str) -> Option<&'static str> {
    if text.eq_ignore_ascii_case("true") || text.eq_ignore_ascii_case("false") {
        return Some("boolean-field");
    }
    if is_integer(text) {
        return Some("integer-field");
    }
    if is_decimal(text) {
        return Some("decimal-field");
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
fn is_decimal(text: &str) -> bool {
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
        return false; // a bare integer is not a decimal
    }
    let Some(exponent) = exponent else {
        return true;
    };
    let digits = exponent.strip_prefix(['-', '+']).unwrap_or(exponent);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn tokenization(parsed: &Parse<'_>, semantic: bool) -> HostTokenization {
    let source = parsed.lexed().source();
    let tokens = parsed
        .lexed()
        .tokens()
        .iter()
        .map(|token| {
            let kind = if semantic {
                semantic_host_kind(*token, source)
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
    options: Options,
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
    Ok(tokenization(&parse(source, options), semantic))
}

fn diagnose(
    descriptor: &LanguageDescriptor,
    options: Options,
    source: &str,
    host: &HostAnalysisOptions,
) -> Result<Vec<HostDiagnostic>, HostError> {
    Ok(analyze(descriptor, options, source, host, true)?.diagnostics)
}

/// CSV host adapter: comma-delimited, RFC 4180 quoting.
#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

/// TSV host adapter: tab-delimited, quoting is not meaningful.
#[derive(Debug, Default, Clone, Copy)]
pub struct TsvHost;

pub static ENGINE: Host = Host;

pub static TSV_ENGINE: TsvHost = TsvHost;

/// LEX, PARSE, SEMANTIC and VALIDATE are real; there is no node-identity
/// tree, cursor, or visitor here, so none of those capabilities are
/// advertised.
pub static DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::CSV,
    "CSV",
    &["csv"],
    &[".csv"],
    &["text/csv"],
    env!("CARGO_PKG_VERSION"),
    CAPABILITIES,
);

/// The same capability surface as CSV, under the TSV identity.
pub static TSV_DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::TSV,
    "TSV",
    &["tsv"],
    &[".tsv"],
    &["text/tab-separated-values"],
    env!("CARGO_PKG_VERSION"),
    CAPABILITIES,
);

impl HostLanguage for Host {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        analyze(&DESCRIPTOR, Options::CSV, source, opts, false)
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        analyze(&DESCRIPTOR, Options::CSV, source, opts, true)
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        diagnose(&DESCRIPTOR, Options::CSV, source, opts)
    }
}

impl HostLanguage for TsvHost {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &TSV_DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        analyze(&TSV_DESCRIPTOR, Options::TSV, source, opts, false)
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        analyze(&TSV_DESCRIPTOR, Options::TSV, source, opts, true)
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        diagnose(&TSV_DESCRIPTOR, Options::TSV, source, opts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every lex-layer kind the engine may emit; the semantic layer only
    /// substitutes the four field readings for `field`.
    const LEX_VOCABULARY: [&str; 8] = [
        "bom",
        "header-field",
        "field",
        "quote",
        "escaped-quote",
        "delimiter",
        "record-break",
        "error",
    ];

    const SAMPLE: &str = concat!(
        "id,name,score,active,notes\n",
        "1,\"Doe, Jane\",42.5,true,\"said \"\"hi\"\"\"\n",
        "2,Zero,007,FALSE,\n",
    );

    fn distinct<'a>(kinds: &'a [&'a str]) -> Vec<&'a str> {
        let mut names: Vec<&str> = kinds.iter().map(|kind| kind.as_ref()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    #[test]
    fn host_lex_emits_only_the_csv_vocabulary() {
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
            "header-field",
            "field",
            "quote",
            "escaped-quote",
            "delimiter",
            "record-break",
        ] {
            assert!(names.contains(&expected), "missing {expected}: {names:?}");
        }
        for name in &names {
            assert!(LEX_VOCABULARY.contains(name), "off-vocabulary {name}");
        }
    }

    #[test]
    fn host_lex_kinds_are_exact_for_a_two_row_file() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.lex("name,score\nada,42\n", &opts).unwrap();
        let names: Vec<&str> = tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect();
        assert_eq!(
            names,
            vec![
                "header-field",
                "delimiter",
                "header-field",
                "record-break",
                "field",
                "delimiter",
                "field",
                "record-break",
            ]
        );
    }

    #[test]
    fn semantic_tokens_retag_fields_without_moving_spans() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.semantic_tokens(SAMPLE, &opts).unwrap();
        assert!(tokenization.valid);
        let kinds: Vec<&str> = tokenization
            .tokens
            .iter()
            .map(|t| t.kind.as_ref())
            .collect();
        let names = distinct(&kinds);
        for expected in [
            "quoted-field",
            "integer-field",
            "decimal-field",
            "boolean-field",
            "header-field",
            "delimiter",
            "record-break",
        ] {
            assert!(names.contains(&expected), "missing {expected}: {names:?}");
        }
        assert!(!names.contains(&"number"), "{names:?}");
        assert!(!names.contains(&"string"), "{names:?}");
        // The exact reading of the third record: 2, Zero, 007 (quoted-ish?
        // no: bare 007 is an integer), FALSE, and one empty field.
        let simple = ENGINE
            .semantic_tokens("a,b\n1,2.5\ntrue,\"true\"\n", &opts)
            .unwrap();
        let kinds: Vec<&str> = simple.tokens.iter().map(|t| t.kind.as_ref()).collect();
        assert_eq!(
            kinds,
            vec![
                "header-field",
                "delimiter",
                "header-field",
                "record-break",
                "integer-field",
                "delimiter",
                "decimal-field",
                "record-break",
                "boolean-field",
                "delimiter",
                "quote",
                "quoted-field",
                "quote",
                "record-break",
            ]
        );
        let syntax = ENGINE.lex("a,b\n1,2.5\ntrue,\"true\"\n", &opts).unwrap();
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
    }

    #[test]
    fn tsv_adapter_treats_quotes_and_commas_as_field_text() {
        let opts = HostAnalysisOptions::default();
        let source = "id\tname\n7\t\"quoted, look: commas\"\n";
        let tokenization = TSV_ENGINE.lex(source, &opts).unwrap();
        assert!(tokenization.valid, "{:?}", tokenization.diagnostics);
        assert!(tokenization.diagnostics.is_empty());
        let names: Vec<&str> = tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect();
        assert_eq!(
            names,
            vec![
                "header-field",
                "delimiter",
                "header-field",
                "record-break",
                "field",
                "delimiter",
                "field",
                "record-break",
            ],
            "a TSV quote is field text, never a quote kind"
        );
        let quoted_field = tokenization
            .tokens
            .iter()
            .find(|token| token.span == HostSpan::from(Span::new(10, 32)))
            .expect("the text field");
        assert_eq!(
            &source[quoted_field.span.start..quoted_field.span.end],
            "\"quoted, look: commas\""
        );
        // The comma row would fall apart under CSV rules but not TSV's.
        assert_eq!(parse(source, Options::TSV).records().len(), 2);
    }

    #[test]
    fn host_diagnose_reports_only_csv_codes() {
        let opts = HostAnalysisOptions::default();
        let codes = |engine: &dyn HostLanguage, source: &str| -> Vec<String> {
            engine
                .diagnose(source, &opts)
                .unwrap()
                .iter()
                .map(|diagnostic| diagnostic.code.to_string())
                .collect()
        };
        assert_eq!(
            codes(&ENGINE, "h\n\"oops"),
            vec!["unclosed-quote".to_string()]
        );
        assert_eq!(
            codes(&ENGINE, "a,\"x\"y,h\n"),
            vec!["text-after-closing-quote".to_string()]
        );
        assert_eq!(
            codes(&ENGINE, "h,h\na,b,c\n"),
            vec!["ragged-row".to_string()]
        );
        assert_eq!(codes(&ENGINE, "h\ta\nb\ta\n"), Vec::<String>::new());
        // TSV reports nothing but ragged rows: quotes cannot be unclosed.
        assert_eq!(
            codes(&TSV_ENGINE, "a\tb\n\"oops\n"),
            vec!["ragged-row".to_string()]
        );
    }

    #[test]
    fn both_descriptors_advertise_only_the_real_surface() {
        for descriptor in [&DESCRIPTOR, &TSV_DESCRIPTOR] {
            assert_eq!(descriptor.capabilities, CAPABILITIES);
            assert!(descriptor.capabilities.contains(
                Capabilities::LEX
                    | Capabilities::PARSE
                    | Capabilities::SEMANTIC
                    | Capabilities::VALIDATE
            ));
            assert!(!descriptor.capabilities.contains(Capabilities::CST));
            assert!(!descriptor.capabilities.contains(Capabilities::NAVIGATE));
            assert!(!descriptor.capabilities.contains(Capabilities::VISITOR));
            assert_eq!(descriptor.engine_version, env!("CARGO_PKG_VERSION"));
        }
        assert_eq!(DESCRIPTOR.extensions, [".csv"]);
        assert_eq!(DESCRIPTOR.mime_types, ["text/csv"]);
        assert_eq!(TSV_DESCRIPTOR.extensions, [".tsv"]);
        assert_eq!(TSV_DESCRIPTOR.mime_types, ["text/tab-separated-values"]);
    }

    #[test]
    fn host_rejects_oversized_input() {
        let opts = HostAnalysisOptions::default().with_limits(
            themoretheless_tokenizer_core::InputLimits::conservative().max_input_bytes(4),
        );
        for engine in [&ENGINE as &dyn HostLanguage, &TSV_ENGINE] {
            assert!(matches!(
                engine.lex("a,b,c\n", &opts),
                Err(HostError::InputTooLarge { .. })
            ));
        }
    }

    #[test]
    fn field_classification_is_exhaustive() {
        for (text, expected) in [
            ("42", "integer-field"),
            ("-7", "integer-field"),
            ("+7", "field"),
            ("0", "integer-field"),
            ("4.2", "decimal-field"),
            ("-1.5e3", "decimal-field"),
            (".5", "decimal-field"),
            ("5.", "decimal-field"),
            ("2e10", "decimal-field"),
            ("e5", "field"),
            ("", "field"),
            ("true", "boolean-field"),
            ("FALSE", "boolean-field"),
            ("TrUe", "boolean-field"),
            ("truex", "field"),
            ("007", "integer-field"),
            ("1:2", "field"),
            ("1..2", "field"),
            ("1e", "field"),
        ] {
            let source = format!("h\n{text},h\n");
            let tokenization = ENGINE
                .semantic_tokens(&source, &HostAnalysisOptions::default())
                .unwrap();
            let kind = tokenization
                .tokens
                .iter()
                .find(|token| token.span == HostSpan::from(Span::new(2, 2 + text.len())))
                .map(|token| token.kind.as_ref());
            if text.is_empty() {
                assert!(kind.is_none(), "empty field must produce no token");
                continue;
            }
            assert_eq!(kind, Some(expected), "for {text:?}");
        }
    }

    #[test]
    fn quoted_numbers_stay_text_in_the_semantic_layer() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.semantic_tokens("a,b\n\"007\",42\n", &opts).unwrap();
        assert!(tokenization.valid);
        let names: Vec<&str> = tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect();
        assert_eq!(
            names,
            vec![
                "header-field",
                "delimiter",
                "header-field",
                "record-break",
                "quote",
                "quoted-field",
                "quote",
                "delimiter",
                "integer-field",
                "record-break",
            ]
        );
    }
}
