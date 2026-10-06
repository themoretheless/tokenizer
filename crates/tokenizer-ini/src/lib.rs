//! A lossless INI and Java `.properties` engine: one dialect-parameterised
//! lexer, a recovering section/entry pass, and a value-aware semantic layer.
//!
//! `.properties` is a thin dialect of the same shape, so both engines live
//! here and differ only through [`Options`] — on rules that change what the
//! bytes mean, not just what they are called. `[a]` is a section header to INI
//! and an ordinary key to `.properties`; `!c` is a comment only in
//! `.properties` and `;c` only in INI; a whitespace run separates key from
//! value in `.properties` and is part of the key in INI. The engine owns its
//! vocabulary — there is no `identifier`, no `string` and no `number` kind,
//! because in these formats the structural names are `key`, `value`,
//! `separator` and `section-marker`. Concatenating every token's text
//! reconstructs the source byte-for-byte, including for malformed input: bad
//! spans are flagged, never dropped or synthesized.
//!
//! ```
//! use themoretheless_tokenizer_ini::{Options, SyntaxKind, parse};
//!
//! let source = "[owner]\nname = Grace\nlazy = true\n";
//! let parsed = parse(source, Options::INI);
//! assert!(parsed.is_valid());
//! assert_eq!(parsed.sections().len(), 1);
//! assert_eq!(parsed.entries().len(), 2);
//! assert_eq!(parsed.lexed().joined(), source);
//!
//! let kinds = parsed.lexed().tokens().iter().map(|t| t.kind).collect::<Vec<_>>();
//! assert!(kinds.contains(&SyntaxKind::SectionMarker));
//! assert!(kinds.contains(&SyntaxKind::SectionName));
//!
//! // The same bytes under .properties rules are three flat entries and no
//! // section at all.
//! let flat = parse(source, Options::PROPERTIES);
//! assert!(flat.sections().is_empty());
//! assert_eq!(flat.entries().len(), 3);
//! ```
//!
//! The host adapters add the format-specific wire kinds, and the semantic
//! layer re-tags a value region that is one complete unquoted span by what it
//! holds (`integer-value`, `decimal-value`, `boolean-value`) while keeping
//! every span identical, so a quoted `"007"`, a `1\t2` split by an escape, and
//! a path like `C:\INI\cfg` all stay text.
//!
//! ```
//! use themoretheless_tokenizer_ini::{ENGINE, PROPERTIES_ENGINE};
//! use themoretheless_tokenizer_core::{HostAnalysisOptions, HostLanguage};
//!
//! let opts = HostAnalysisOptions::default();
//! let kinds = |engine: &dyn HostLanguage, source: &str| -> Vec<String> {
//!     engine
//!         .lex(source, &opts)
//!         .unwrap()
//!         .tokens
//!         .iter()
//!         .map(|token| token.kind.to_string())
//!         .collect()
//! };
//! assert_eq!(
//!     kinds(&ENGINE, "[a]"),
//!     vec!["section-marker", "section-name", "section-marker"]
//! );
//! assert_eq!(kinds(&PROPERTIES_ENGINE, "[a]"), vec!["key"]);
//! assert_eq!(kinds(&PROPERTIES_ENGINE, "!c"), vec!["comment"]);
//! assert_eq!(kinds(&ENGINE, "!c"), vec!["key"]);
//! ```

#![forbid(unsafe_code)]

mod lexer;
mod parser;

pub use lexer::{Dialect, LexToken, Lexed, Options, SyntaxKind, TokenFlags, lex};

pub use parser::{DiagnosticKind, Entry, Parse, Section, parse, validate};

pub use themoretheless_tokenizer_core::{Diagnostic, Span};

// ─── Host adapters ───────────────────────────────────────────────────────────

use std::borrow::Cow;
use std::collections::HashSet;

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

/// Host token kind: the config-format vocabulary, so an editor can tell a
/// section header from a key and a separator from punctuation.
fn lex_host_kind(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::Bom => "bom",
        SyntaxKind::SectionMarker => "section-marker",
        SyntaxKind::SectionName => "section-name",
        SyntaxKind::Key => "key",
        SyntaxKind::Separator => "separator",
        SyntaxKind::Value => "value",
        SyntaxKind::Quote => "quote",
        SyntaxKind::Comment => "comment",
        SyntaxKind::EscapeSequence => "escape-sequence",
        SyntaxKind::LineContinuation => "line-continuation",
        SyntaxKind::Padding => "padding",
        SyntaxKind::RecordBreak => "record-break",
        SyntaxKind::Error => "error",
    }
}

/// The value spans a value region may be read from: exactly one unquoted,
/// unflagged span per entry. The semantic layer may type only these.
fn typeable_values(parsed: &Parse<'_>) -> HashSet<Span> {
    parsed
        .entries()
        .iter()
        .filter(|entry| !entry.has_error && !entry.value_quoted)
        .filter_map(|entry| entry.value)
        .collect()
}

/// Semantic layer: a quoted span becomes `quoted-key` or `quoted-value`, and a
/// value region that is one complete unquoted span is re-tagged by what it
/// holds. Section, key, separator, comment, continuation, padding and break
/// kinds keep their syntax-layer name, and the span never moves.
fn semantic_host_kind(token: LexToken, source: &str, typeable: &HashSet<Span>) -> &'static str {
    if token.has_error() {
        return "error";
    }
    if token.is_quoted() {
        return match token.kind {
            SyntaxKind::Key => "quoted-key",
            SyntaxKind::Value => "quoted-value",
            _ => lex_host_kind(token.kind),
        };
    }
    if token.kind == SyntaxKind::Value && typeable.contains(&token.span) {
        return classify_bare_value(token.text(source).unwrap_or_default());
    }
    lex_host_kind(token.kind)
}

/// What an unquoted, single-span value's bytes say they are. A quoted or
/// fragmented value never reaches here, so `C:\INI` and `1\t2` stay text.
fn classify_bare_value(text: &str) -> &'static str {
    if text.eq_ignore_ascii_case("true") || text.eq_ignore_ascii_case("false") {
        return "boolean-value";
    }
    if is_integer(text) {
        return "integer-value";
    }
    if is_decimal(text) {
        return "decimal-value";
    }
    "value"
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
    let typeable = typeable_values(parsed);
    let tokens = parsed
        .lexed()
        .tokens()
        .iter()
        .map(|token| {
            let kind = if semantic {
                semantic_host_kind(*token, source, &typeable)
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
            severity: DiagnosticKind::from_code(diagnostic.code)
                .map_or(Severity::Error, |kind| kind.severity()),
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

/// INI host adapter: `[section]` headers, `key = value`, `;`/`#` comments.
#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

/// `.properties` host adapter: flat keys, `#`/`!` comments, backslash escapes.
#[derive(Debug, Default, Clone, Copy)]
pub struct PropertiesHost;

pub static ENGINE: Host = Host;

pub static PROPERTIES_ENGINE: PropertiesHost = PropertiesHost;

/// LEX, PARSE, SEMANTIC and VALIDATE are real; there is no node-identity
/// tree, cursor, or visitor here, so none of those capabilities are
/// advertised.
pub static DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::INI,
    "INI",
    &["ini", "config"],
    &[".ini", ".cfg"],
    &["text/plain"],
    env!("CARGO_PKG_VERSION"),
    CAPABILITIES,
);

/// The same capability surface as INI, under the `.properties` identity.
pub static PROPERTIES_DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::PROPERTIES,
    "Properties",
    &["properties", "java-properties"],
    &[".properties"],
    &["text/x-java-properties"],
    env!("CARGO_PKG_VERSION"),
    CAPABILITIES,
);

impl HostLanguage for Host {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        analyze(&DESCRIPTOR, Options::INI, source, opts, false)
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        analyze(&DESCRIPTOR, Options::INI, source, opts, true)
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        diagnose(&DESCRIPTOR, Options::INI, source, opts)
    }
}

impl HostLanguage for PropertiesHost {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        &PROPERTIES_DESCRIPTOR
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        analyze(
            &PROPERTIES_DESCRIPTOR,
            Options::PROPERTIES,
            source,
            opts,
            false,
        )
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        analyze(
            &PROPERTIES_DESCRIPTOR,
            Options::PROPERTIES,
            source,
            opts,
            true,
        )
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        diagnose(&PROPERTIES_DESCRIPTOR, Options::PROPERTIES, source, opts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind either dialect may emit at the lex layer.
    const LEX_VOCABULARY: [&str; 13] = [
        "bom",
        "section-marker",
        "section-name",
        "key",
        "separator",
        "value",
        "quote",
        "comment",
        "escape-sequence",
        "line-continuation",
        "padding",
        "record-break",
        "error",
    ];

    /// Kinds INI can never produce: it has no escape processing.
    const INI_FORBIDDEN: [&str; 1] = ["escape-sequence"];

    /// Kinds `.properties` can never produce: no sections, no quoting.
    const PROPERTIES_FORBIDDEN: [&str; 3] = ["section-marker", "section-name", "quote"];

    const SAMPLE: &str = concat!(
        "; config for the demo\n",
        "[server]\n",
        "host = example.com\n",
        "port = 8080\n",
        "secure = true\n",
        "banner = \"welcome, all\"\n",
        "notes = line one\n",
        "  line two\n",
        "path = C:\\INI\\cfg\n",
    );

    const PROPERTIES_SAMPLE: &str = concat!(
        "# build settings\n",
        "! and a bang comment\n",
        "group.id = 42\n",
        "ratio = 1.5e2\n",
        "name\\:full = Grace\\tHopper\n",
        "wrapped = a\\\n",
        "        b\n",
        "version = -7\n",
    );

    fn kinds_of(tokenization: &HostTokenization) -> Vec<&str> {
        tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect()
    }

    fn names(engine: &dyn HostLanguage, source: &str, semantic: bool) -> Vec<String> {
        let opts = HostAnalysisOptions::default();
        let tokenization = if semantic {
            engine.semantic_tokens(source, &opts)
        } else {
            engine.lex(source, &opts)
        }
        .unwrap();
        tokenization
            .tokens
            .iter()
            .map(|token| token.kind.to_string())
            .collect()
    }

    fn assert_in_vocabulary(names: &[String], forbidden: &[&str]) {
        let semantic_only = [
            "quoted-key",
            "quoted-value",
            "integer-value",
            "decimal-value",
            "boolean-value",
        ];
        for name in names {
            assert!(
                LEX_VOCABULARY.contains(&name.as_str()) || semantic_only.contains(&name.as_str()),
                "off-vocabulary kind {name}"
            );
            assert!(!forbidden.contains(&name.as_str()), "forbidden {name}");
        }
    }

    #[test]
    fn host_lex_emits_the_ini_vocabulary_only() {
        let found = names(&ENGINE, SAMPLE, false);
        for expected in [
            "comment",
            "section-marker",
            "section-name",
            "key",
            "separator",
            "value",
            "quote",
            "line-continuation",
            "padding",
            "record-break",
        ] {
            assert!(found.contains(&expected.to_string()), "missing {expected}");
        }
        assert_in_vocabulary(&found, &INI_FORBIDDEN);
        assert!(!found.contains(&"error".to_string()), "the sample is clean");
        assert!(
            !found
                .iter()
                .any(|kind| *kind == "identifier" || *kind == "string")
        );
    }

    #[test]
    fn host_lex_emits_the_properties_vocabulary_only() {
        let found = names(&PROPERTIES_ENGINE, PROPERTIES_SAMPLE, false);
        for expected in [
            "comment",
            "key",
            "separator",
            "value",
            "escape-sequence",
            "line-continuation",
            "padding",
            "record-break",
        ] {
            assert!(found.contains(&expected.to_string()), "missing {expected}");
        }
        assert_in_vocabulary(&found, &PROPERTIES_FORBIDDEN);
    }

    #[test]
    fn ini_entry_kinds_are_exact_through_the_host() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.lex("[s]\nk = v\n", &opts).unwrap();
        assert!(tokenization.valid);
        assert!(tokenization.diagnostics.is_empty());
        assert_eq!(
            kinds_of(&tokenization),
            vec![
                "section-marker",
                "section-name",
                "section-marker",
                "record-break",
                "key",
                "padding",
                "separator",
                "padding",
                "value",
                "record-break",
            ]
        );
    }

    #[test]
    fn properties_key_kinds_are_exact_through_the_host() {
        let opts = HostAnalysisOptions::default();
        let tokenization = PROPERTIES_ENGINE.lex("a\\:b c\n", &opts).unwrap();
        assert_eq!(
            kinds_of(&tokenization),
            vec![
                "key",
                "escape-sequence",
                "key",
                "separator",
                "value",
                "record-break",
            ]
        );
    }

    #[test]
    fn semantic_tokens_retag_values_without_moving_spans() {
        let opts = HostAnalysisOptions::default();
        let source = "[s]\nport = 8080\nratio = 1.5e2\nlazy = true\nname = \"grace\"\n";
        let syntax = ENGINE.lex(source, &opts).unwrap();
        let semantic = ENGINE.semantic_tokens(source, &opts).unwrap();
        assert!(syntax.valid && semantic.valid);
        assert_eq!(
            syntax
                .tokens
                .iter()
                .map(|token| token.span)
                .collect::<Vec<_>>(),
            semantic
                .tokens
                .iter()
                .map(|token| token.span)
                .collect::<Vec<_>>(),
            "the semantic layer must keep the syntax-layer spans"
        );
        let found = kinds_of(&semantic);
        for expected in [
            "integer-value",
            "decimal-value",
            "boolean-value",
            "quoted-value",
            "section-name",
            "separator",
        ] {
            assert!(found.contains(&expected), "missing {expected}");
        }
        assert!(!found.contains(&"number"), "{found:?}");
        assert!(!found.contains(&"string"), "{found:?}");
        assert_eq!(
            found,
            vec![
                "section-marker",
                "section-name",
                "section-marker",
                "record-break",
                "key",
                "padding",
                "separator",
                "padding",
                "integer-value",
                "record-break",
                "key",
                "padding",
                "separator",
                "padding",
                "decimal-value",
                "record-break",
                "key",
                "padding",
                "separator",
                "padding",
                "boolean-value",
                "record-break",
                "key",
                "padding",
                "separator",
                "padding",
                "quote",
                "quoted-value",
                "quote",
                "record-break",
            ]
        );
    }

    #[test]
    fn quoted_and_fragmented_values_stay_text() {
        let opts = HostAnalysisOptions::default();
        // A value split by an escape is not one number …
        let properties = PROPERTIES_ENGINE
            .semantic_tokens("k = 1\\t2\nn = 42\n", &opts)
            .unwrap();
        let found = kinds_of(&properties);
        assert_eq!(
            found
                .iter()
                .filter(|kind| **kind == "integer-value")
                .count(),
            1,
            "only the whole-span 42 reads as an integer"
        );
        // … and a quoted key keeps its quotes as separate tokens.
        let ini = ENGINE.semantic_tokens("[s]\n\"k\" = 1\n", &opts).unwrap();
        let found = kinds_of(&ini);
        assert_eq!(
            found,
            vec![
                "section-marker",
                "section-name",
                "section-marker",
                "record-break",
                "quote",
                "quoted-key",
                "quote",
                "padding",
                "separator",
                "padding",
                "integer-value",
                "record-break",
            ]
        );
    }

    #[test]
    fn value_classification_is_exhaustive() {
        for (text, expected) in [
            ("42", "integer-value"),
            ("-7", "integer-value"),
            ("+7", "value"),
            ("0", "integer-value"),
            ("4.2", "decimal-value"),
            ("-1.5e3", "decimal-value"),
            (".5", "decimal-value"),
            ("5.", "decimal-value"),
            ("2e10", "decimal-value"),
            ("e5", "value"),
            ("", "value"),
            ("true", "boolean-value"),
            ("FALSE", "boolean-value"),
            ("TrUe", "boolean-value"),
            ("truex", "value"),
            ("007", "integer-value"),
            ("1:2", "value"),
            ("C:\\INI\\cfg", "value"),
            ("1906-12-09", "value"),
            ("-O2;-g", "value"),
            ("1..2", "value"),
            ("1e", "value"),
        ] {
            let source = format!("[s]\nk = {text}\n");
            let tokenization = ENGINE
                .semantic_tokens(&source, &HostAnalysisOptions::default())
                .unwrap();
            let kind = tokenization
                .tokens
                .iter()
                .find(|token| token.span == HostSpan::from(Span::new(8, 8 + text.len())))
                .map(|token| token.kind.as_ref());
            if text.is_empty() {
                assert!(kind.is_none(), "an empty value produces no token");
                continue;
            }
            assert_eq!(kind, Some(expected), "for {text:?}");
        }
    }

    #[test]
    fn the_dialects_disagree_in_both_directions() {
        let ini = |source: &str| names(&ENGINE, source, false);
        let properties = |source: &str| names(&PROPERTIES_ENGINE, source, false);

        // A section header is structure in INI and a plain key in properties.
        assert_eq!(
            ini("[a]"),
            vec!["section-marker", "section-name", "section-marker"]
        );
        assert_eq!(properties("[a]"), vec!["key"]);

        // `!` comments only in properties, `;` only in INI.
        assert_eq!(properties("!c"), vec!["comment"]);
        assert_eq!(ini("!c"), vec!["key"]);
        assert_eq!(ini(";c"), vec!["comment"]);
        assert_eq!(properties(";c"), vec!["key"]);

        // Whitespace and colon separate only in properties.
        assert_eq!(properties("a b"), vec!["key", "separator", "value"]);
        assert_eq!(ini("a b"), vec!["key"]);
        assert_eq!(properties("a:b"), vec!["key", "separator", "value"]);
        assert_eq!(ini("a:b"), vec!["key"]);

        // Continuations: indented lines in INI, backslashes in properties.
        assert!(ini("k = 1\n  two\n").contains(&"line-continuation".to_string()));
        assert!(
            !properties("k = 1\n  two\n").contains(&"line-continuation".to_string()),
            "an indent is a new entry in properties"
        );
        assert!(
            properties("k = 1\\\nnext = 2\n").contains(&"line-continuation".to_string()),
            "a backslash joins the next line only in properties"
        );
        assert!(
            !ini("k = 1\\\nnext = 2\n").contains(&"line-continuation".to_string()),
            "the same trailing backslash is INI value text, so `next` starts fresh"
        );

        // Escapes and quotes belong to opposite dialects.
        assert!(
            properties("k = a\\tb").contains(&"escape-sequence".to_string()),
            "properties decodes escapes into their own tokens"
        );
        assert!(
            !ini("k = a\\tb").contains(&"escape-sequence".to_string()),
            "INI keeps the backslash inside the value"
        );
        assert!(ini("k = \"a;b\"").contains(&"quote".to_string()));
        assert!(!properties("k = \"a;b\"").contains(&"quote".to_string()));
        assert_eq!(
            ini("k = \"a;b\""),
            vec![
                "key",
                "padding",
                "separator",
                "padding",
                "quote",
                "value",
                "quote"
            ]
        );
        assert_eq!(
            properties("k = \"a;b\""),
            vec!["key", "padding", "separator", "padding", "value"]
        );
    }

    #[test]
    fn host_diagnose_reports_stable_codes_with_real_severities() {
        let opts = HostAnalysisOptions::default();
        let reports = |engine: &dyn HostLanguage, source: &str| -> Vec<(String, Severity)> {
            engine
                .diagnose(source, &opts)
                .unwrap()
                .iter()
                .map(|diagnostic| (diagnostic.code.to_string(), diagnostic.severity))
                .collect()
        };
        assert_eq!(
            reports(&ENGINE, "[oops\n"),
            vec![("unterminated-section".to_string(), Severity::Error)]
        );
        assert_eq!(
            reports(&ENGINE, "[a]\n[a]\n"),
            vec![("duplicate-section".to_string(), Severity::Warning)]
        );
        assert_eq!(
            reports(&ENGINE, "k = 1\n"),
            vec![("key-outside-section".to_string(), Severity::Warning)]
        );
        assert_eq!(
            reports(&ENGINE, "[s]\nk = \"v\n"),
            vec![("unclosed-quote".to_string(), Severity::Error)]
        );
        assert_eq!(
            reports(&ENGINE, "[s]\nk = \"v\" tail\n"),
            vec![("text-after-closing-quote".to_string(), Severity::Error)]
        );
        assert_eq!(
            reports(&ENGINE, "[s] tail\n"),
            vec![("text-after-section-header".to_string(), Severity::Error)]
        );
        assert_eq!(
            reports(&PROPERTIES_ENGINE, "k = \\q\n"),
            vec![("invalid-escape".to_string(), Severity::Warning)]
        );
        // Dialect-appropriate silences: properties has no sections and no
        // quotes, INI has no escapes.
        assert!(reports(&PROPERTIES_ENGINE, "[oops\nk = \"v\n").is_empty());
        assert!(reports(&ENGINE, "[s]\nk = \\q\n").is_empty());
        // And a clean document in either dialect reports nothing at all.
        assert!(reports(&ENGINE, SAMPLE).is_empty());
        assert!(reports(&PROPERTIES_ENGINE, PROPERTIES_SAMPLE).is_empty());
    }

    #[test]
    fn host_tokens_reconstruct_the_source_byte_for_byte() {
        let corpus = [
            SAMPLE,
            PROPERTIES_SAMPLE,
            "",
            "\n",
            "[a",
            "[a]x\n",
            "k=\"unclosed\nnext=1\n",
            "\u{FEFF}[s]\nk = \u{1F600}值\r\nnext = 2\r",
            "a\\:b = \\\n  cont\n",
            "trailing backslash \\",
            "empty=\nonly-key\n",
            "k = v ; c\n  cont line\n",
        ];
        let opts = HostAnalysisOptions::default();
        for source in corpus {
            for engine in [&ENGINE as &dyn HostLanguage, &PROPERTIES_ENGINE] {
                for semantic in [false, true] {
                    let tokenization = if semantic {
                        engine.semantic_tokens(source, &opts).unwrap()
                    } else {
                        engine.lex(source, &opts).unwrap()
                    };
                    let mut joined = String::new();
                    let mut previous = 0usize;
                    for token in &tokenization.tokens {
                        assert!(
                            token.span.end > token.span.start && token.span.start >= previous,
                            "gap, overlap or zero-width span in {source:?}"
                        );
                        assert!(
                            source.is_char_boundary(token.span.start)
                                && source.is_char_boundary(token.span.end),
                            "span off a char boundary in {source:?}"
                        );
                        joined.push_str(&source[token.span.start..token.span.end]);
                        previous = token.span.end;
                    }
                    assert_eq!(joined, source, "{source:?} semantic={semantic}");
                }
            }
        }
    }

    #[test]
    fn both_descriptors_advertise_only_the_real_surface() {
        for descriptor in [&DESCRIPTOR, &PROPERTIES_DESCRIPTOR] {
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
        assert_eq!(DESCRIPTOR.language, LanguageId::INI);
        assert_eq!(PROPERTIES_DESCRIPTOR.language, LanguageId::PROPERTIES);
        assert_eq!(DESCRIPTOR.extensions, [".ini", ".cfg"]);
        assert_eq!(DESCRIPTOR.mime_types, ["text/plain"]);
        assert_eq!(PROPERTIES_DESCRIPTOR.extensions, [".properties"]);
        assert_eq!(PROPERTIES_DESCRIPTOR.mime_types, ["text/x-java-properties"]);
        assert!(ENGINE.require(Capabilities::LEX).is_ok());
        assert!(PROPERTIES_ENGINE.require(Capabilities::CST).is_err());
    }

    #[test]
    fn host_rejects_oversized_input() {
        let opts = HostAnalysisOptions::default().with_limits(
            themoretheless_tokenizer_core::InputLimits::conservative().max_input_bytes(3),
        );
        for engine in [&ENGINE as &dyn HostLanguage, &PROPERTIES_ENGINE] {
            assert!(matches!(
                engine.lex("[s]\n", &opts),
                Err(HostError::InputTooLarge { .. })
            ));
            assert!(engine.diagnose("[s]\n", &opts).is_err());
        }
    }
}
