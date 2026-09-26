//! A lossless iCalendar (`.ics`) engine: lexer, recovering content-line pass,
//! property-aware semantic layer, RFC 5545 validator.
//!
//! iCalendar is a line format, not an expression language. One logical unit is a
//! *content line*, `name *(";" param) ":" value CRLF`, and the only nesting come
//! from `BEGIN:`/`END:` pairs. This engine owns its vocabulary — there is no
//! `keyword`, no `string`, no `number`, no `punctuation`, because `BEGIN` is a
//! `structure-marker`, a property name is a `property-name`, `:` is a
//! `value-delimiter`, and a value is a `value` until the structure pass types it
//! as a `date-value`, a `duration-value` or one of the others.
//!
//! The two passes divide by *physical* versus *logical*:
//!
//! * [`lex`] never unfolds. A fold — `CRLF` plus one space or tab — becomes its
//!   own `fold-marker` token and every other span stays inside one physical line,
//!   so spans are always regions of the raw bytes.
//! * [`parse`] re-joins folded lines to read a value, matches components, types
//!   values by property name (with `VALUE=` overriding) and reports faults with
//!   stable kebab-case codes.
//!
//! Concatenating every token's text reconstructs the source byte-for-byte, for
//! well-formed calendars and for broken ones alike; bad spans are flagged, never
//! dropped or synthesized. Nothing here is calendar *domain* logic: no
//! recurrence expansion, no time-zone resolution, no `DTSTART`/`DUE` consistency
//! rules — `TZID` is a parameter, not a database lookup.
//!
//! ```
//! use themoretheless_tokenizer_ics::{parse, validate};
//!
//! let source = concat!(
//!     "BEGIN:VCALENDAR\r\n",
//!     "VERSION:2.0\r\n",
//!     "PRODID:-//Example//Tokenizer//EN\r\n",
//!     "END:VCALENDAR\r\n",
//! );
//! let parsed = parse(source);
//! assert!(parsed.is_valid());
//! assert_eq!(parsed.components().len(), 1);
//! assert_eq!(parsed.lexed().joined(), source);
//! assert!(parsed.lines().iter().any(|line| line.is_structure));
//!
//! // Recovery keeps every byte and names the fault with a stable code.
//! let codes: Vec<&str> = validate("SUMMARY:no wrapper\r\n").iter().map(|d| d.code).collect();
//! assert_eq!(codes, ["property-before-begin", "missing-vcalendar-wrapper"]);
//! ```
//!
//! The semantic layer retags the bytes after `BEGIN:`/`END:` as
//! `component-name` and types a value region by the property that owns it,
//! without moving a single span.

#![forbid(unsafe_code)]

mod lexer;
mod parser;

pub use lexer::{LexToken, Lexed, SyntaxKind, TokenFlags, lex};

pub use parser::{
    Component, ContentLine, Diagnostic, DiagnosticKind, Parse, ValueType, parse, validate,
};

pub use themoretheless_tokenizer_core::{LosslessViolation, Span, verify_lossless_spans};

use themoretheless_tokenizer_core::LanguageId;

// ─── Host adapters ───────────────────────────────────────────────────────────

use std::borrow::Cow;

use themoretheless_tokenizer_core::{
    Capabilities, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostSpan,
    HostToken, HostTokenization, LanguageDescriptor, language_descriptor, require_default_dialect,
};

/// Real engine surface: no CST, cursor navigation or visitor API exists here.
const CAPABILITIES: Capabilities = Capabilities::LEX
    .union(Capabilities::PARSE)
    .union(Capabilities::SEMANTIC)
    .union(Capabilities::VALIDATE);

/// Lex-layer kind: the content-line vocabulary, so an editor can tell a property
/// name from a parameter name, a quoted parameter value from a bare one, a fold
/// from a line break, and a `:` from a `;`.
pub(crate) fn lex_host_kind(kind: SyntaxKind) -> &'static str {
    match kind {
        SyntaxKind::Bom => "bom",
        SyntaxKind::StructureMarker => "structure-marker",
        SyntaxKind::PropertyName => "property-name",
        SyntaxKind::ParameterName => "parameter-name",
        SyntaxKind::ValueColon => "value-delimiter",
        SyntaxKind::ParameterSeparator => "parameter-delimiter",
        SyntaxKind::ParameterEquals => "parameter-assignment",
        SyntaxKind::ValueComma => "value-list-delimiter",
        SyntaxKind::QuotedParam => "quoted-param-value",
        SyntaxKind::BareParam => "bare-param-value",
        SyntaxKind::Value => "value",
        SyntaxKind::Escape => "escaped-char",
        SyntaxKind::LineBreak => "line-break",
        SyntaxKind::FoldMarker => "fold-marker",
        SyntaxKind::Error => "error",
    }
}

/// Semantic layer: the syntax kinds plus the readings that need the component or
/// the property around a value, applied without moving a span.
///
/// * The bytes after `BEGIN:`/`END:` become `component-name`, not a value.
/// * A value region is typed by the property that owns it, with `VALUE=`
///   overriding: `date-value`, `date-time-value`, `duration-value`,
///   `period-value`, `recurrence-value`, `uri-value`, `text-value`.
///
/// Nothing is invented: a reading only lands on a span the lexer already
/// emitted, so the semantic layer can never claim a token syntax lacks.
fn semantic_host_kind(parsed: &Parse<'_>, token: LexToken) -> &'static str {
    parsed.semantic_kind(token)
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
            severity: diagnostic.severity,
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

/// iCalendar host adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct Host;

pub static ENGINE: Host = Host;

/// LEX, PARSE, SEMANTIC and VALIDATE are real; there is no node-identity tree,
/// cursor or visitor here, so none of those capabilities are advertised.
pub static DESCRIPTOR: LanguageDescriptor = language_descriptor(
    LanguageId::ICS,
    "ics",
    &["ics", "icalendar", "iCalendar"],
    &[".ics"],
    &["text/calendar"],
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
    use themoretheless_tokenizer_core::{DialectId, InputLimits, Severity};

    /// Every lex-layer kind the engine may emit.
    const LEX_VOCABULARY: [&str; 15] = [
        "bom",
        "structure-marker",
        "property-name",
        "parameter-name",
        "value-delimiter",
        "parameter-delimiter",
        "parameter-assignment",
        "value-list-delimiter",
        "quoted-param-value",
        "bare-param-value",
        "value",
        "escaped-char",
        "line-break",
        "fold-marker",
        "error",
    ];

    /// The lex vocabulary plus the eight readings the semantic layer may
    /// substitute; it never adds a kind with no span behind it.
    const SEMANTIC_VOCABULARY: [&str; 23] = [
        "bom",
        "structure-marker",
        "property-name",
        "parameter-name",
        "value-delimiter",
        "parameter-delimiter",
        "parameter-assignment",
        "value-list-delimiter",
        "quoted-param-value",
        "bare-param-value",
        "value",
        "escaped-char",
        "line-break",
        "fold-marker",
        "error",
        "component-name",
        "date-value",
        "date-time-value",
        "duration-value",
        "period-value",
        "recurrence-value",
        "uri-value",
        "text-value",
    ];

    /// The eleven kinds a generated engine would emit for any format.
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

    /// A representative calendar of the shape real producers emit. It must
    /// analyze clean: this is the gate the repository's format suite applies.
    const SAMPLE: &str = concat!(
        "BEGIN:VCALENDAR\r\n",
        "VERSION:2.0\r\n",
        "PRODID:-//Example Corp//Tokenizer ics//EN\r\n",
        "BEGIN:VTIMEZONE\r\n",
        "TZID:Europe/Berlin\r\n",
        "BEGIN:DAYLIGHT\r\n",
        "DTSTART:19810426T020000\r\n",
        "TZOFFSETFROM:+0100\r\n",
        "TZOFFSETTO:+0200\r\n",
        "TZNAME:CEST\r\n",
        "END:DAYLIGHT\r\n",
        "END:VTIMEZONE\r\n",
        "BEGIN:VEVENT\r\n",
        "UID:19970901T100000Z-123401@example.com\r\n",
        "DTSTAMP:20260924T100000Z\r\n",
        "DTSTART;TZID=Europe/Berlin:20260924T090000\r\n",
        "DTEND;VALUE=DATE:20260925\r\n",
        "SUMMARY:Launch party\\, everyone\\nplease bring a drink\r\n",
        "DESCRIPTION:A folded description\r\n\tcontinues on the next line\r\n",
        "CATEGORIES:MEETING,PROJECT X\r\n",
        "ORGANIZER;CN=\"Dana, PM\";SENT-BY=\"mailto:dana@example.com\":mailto:dana@example.com\r\n",
        "ATTENDEE;CN=Ada;DELEGATED-TO=\"mailto:b@example.com\",\"mailto:c@example.com\";ROLE=REQ-PARTICIPANT:mailto:a@example.com\r\n",
        "URL:https://example.com/party\r\n",
        "FREEBUSY:20260924T090000Z/PT1H,20260924T110000Z/20260924T120000Z\r\n",
        "RRULE:FREQ=WEEKLY;UNTIL=20261224T000000Z\r\n",
        "BEGIN:VALARM\r\n",
        "ACTION:DISPLAY\r\n",
        "TRIGGER:-PT15M\r\n",
        "END:VALARM\r\n",
        "END:VEVENT\r\n",
        "END:VCALENDAR\r\n",
    );

    fn names(tokenization: &HostTokenization) -> Vec<&str> {
        tokenization
            .tokens
            .iter()
            .map(|token| token.kind.as_ref())
            .collect()
    }

    fn distinct(mut collected: Vec<&str>) -> Vec<&str> {
        collected.sort_unstable();
        collected.dedup();
        collected
    }

    fn host_joins(source: &str, tokenization: &HostTokenization) -> bool {
        let mut joined = String::new();
        for token in &tokenization.tokens {
            joined.push_str(&source[token.span.start..token.span.end]);
        }
        joined == source
    }

    fn host_spans(tokenization: &HostTokenization) -> Vec<Span> {
        tokenization
            .tokens
            .iter()
            .map(|token| Span::new(token.span.start, token.span.end))
            .collect()
    }

    /// A line placed inside an otherwise conformant calendar, so the only
    /// diagnostic it can raise is the one under test.
    fn wrapped(body: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\n{body}END:VCALENDAR\r\n")
    }

    fn codes(source: &str) -> Vec<&'static str> {
        validate(source)
            .iter()
            .copied()
            .map(Diagnostic::code)
            .collect()
    }

    /// The single semantic reading of the one token whose whole text is `text`.
    fn semantic_of(parsed: &Parse<'_>, text: &str) -> &'static str {
        let source = parsed.lexed().source();
        let token = parsed
            .lexed()
            .tokens()
            .iter()
            .copied()
            .find(|token| token.text(source) == Some(text))
            .unwrap_or_else(|| panic!("no token whose whole text is {text:?}"));
        parsed.semantic_kind(token)
    }

    #[test]
    fn sample_calendar_is_diagnostic_free_on_both_layers() {
        let opts = HostAnalysisOptions::default();
        let syntax = ENGINE.lex(SAMPLE, &opts).unwrap();
        let semantic = ENGINE.semantic_tokens(SAMPLE, &opts).unwrap();
        for (layer, tokenization) in [("syntax", &syntax), ("semantic", &semantic)] {
            assert!(tokenization.valid, "{layer} rejected a valid document");
            assert!(
                tokenization.diagnostics.is_empty(),
                "{layer}: {:?}",
                tokenization.diagnostics
            );
            assert!(host_joins(SAMPLE, tokenization), "{layer} lost bytes");
        }
        let parsed = parse(SAMPLE);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        assert!(parsed.errors().next().is_none());
        assert!(parsed.warnings().next().is_none());
        assert!(parsed.lexed().verify_lossless().is_ok());
        assert_eq!(parsed.components().len(), 5);
    }

    #[test]
    fn lex_layer_emits_only_the_icalendar_vocabulary() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.lex(SAMPLE, &opts).unwrap();
        let kinds = distinct(names(&tokenization));
        for expected in [
            "structure-marker",
            "property-name",
            "parameter-name",
            "value-delimiter",
            "parameter-delimiter",
            "parameter-assignment",
            "value-list-delimiter",
            "quoted-param-value",
            "bare-param-value",
            "value",
            "escaped-char",
            "fold-marker",
            "line-break",
        ] {
            assert!(kinds.contains(&expected), "missing {expected}: {kinds:?}");
        }
        for kind in &kinds {
            assert!(LEX_VOCABULARY.contains(kind), "off-vocabulary {kind}");
        }
        for banned in GENERIC_KINDS {
            assert!(!kinds.contains(&banned), "{banned} is not iCalendar");
        }
    }

    #[test]
    fn semantic_layer_emits_only_the_icalendar_vocabulary() {
        let opts = HostAnalysisOptions::default();
        let tokenization = ENGINE.semantic_tokens(SAMPLE, &opts).unwrap();
        let kinds = distinct(names(&tokenization));
        for expected in [
            "component-name",
            "date-value",
            "date-time-value",
            "duration-value",
            "period-value",
            "recurrence-value",
            "uri-value",
            "text-value",
            "property-name",
            "parameter-name",
        ] {
            assert!(kinds.contains(&expected), "missing {expected}: {kinds:?}");
        }
        for kind in &kinds {
            assert!(SEMANTIC_VOCABULARY.contains(kind), "off-vocabulary {kind}");
        }
        for banned in GENERIC_KINDS {
            assert!(!kinds.contains(&banned), "{banned} is not iCalendar");
        }
        let semantic = ENGINE.semantic_tokens(SAMPLE, &opts).unwrap();
        let specific: Vec<&str> = names(&semantic)
            .into_iter()
            .filter(|kind| !GENERIC_KINDS.contains(kind))
            .collect();
        assert!(
            distinct(specific.clone()).len() >= 8,
            "only {} specific kinds",
            distinct(specific).len()
        );
    }

    /// The exact vocabularies the sample produces, measured from the engine.
    #[test]
    fn exact_sorted_kind_sets_for_the_sample() {
        let opts = HostAnalysisOptions::default();
        let syntax = ENGINE.lex(SAMPLE, &opts).unwrap();
        let semantic = ENGINE.semantic_tokens(SAMPLE, &opts).unwrap();
        assert_eq!(
            distinct(names(&syntax)),
            vec![
                "bare-param-value",
                "escaped-char",
                "fold-marker",
                "line-break",
                "parameter-assignment",
                "parameter-delimiter",
                "parameter-name",
                "property-name",
                "quoted-param-value",
                "structure-marker",
                "value",
                "value-delimiter",
                "value-list-delimiter",
            ]
        );
        let expected_semantic: Vec<&str> = vec![
            "bare-param-value",
            "component-name",
            "date-time-value",
            "date-value",
            "duration-value",
            "escaped-char",
            "fold-marker",
            "line-break",
            "parameter-assignment",
            "parameter-delimiter",
            "parameter-name",
            "period-value",
            "property-name",
            "quoted-param-value",
            "recurrence-value",
            "structure-marker",
            "text-value",
            "uri-value",
            "value-delimiter",
            "value-list-delimiter",
        ];
        assert_eq!(distinct(names(&semantic)), expected_semantic);
        // The fixture's non-generic semantic vocabulary, measured: every kind the
        // engine emits is format-specific, so filtering the generic set out of the
        // list above changes nothing.
        let specific = ENGINE.semantic_tokens(SAMPLE, &opts).unwrap();
        let mut filtered: Vec<&str> = names(&specific)
            .into_iter()
            .filter(|kind| !GENERIC_KINDS.contains(kind))
            .collect();
        filtered.sort_unstable();
        filtered.dedup();
        assert_eq!(filtered, expected_semantic);
        assert!(filtered.len() >= 8);
        assert_eq!(syntax.tokens.len(), semantic.tokens.len());
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
            "the semantic layer must not move a span"
        );
    }

    #[test]
    fn value_typing_follows_the_property_and_the_value_param() {
        let source = concat!(
            "BEGIN:VCALENDAR\r\n",
            "VERSION:2.0\r\n",
            "PRODID:x\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:1\r\n",
            "DTSTAMP:20260924T100000Z\r\n",
            "DTSTART;TZID=Europe/Berlin:20260924T090000\r\n",
            "DTEND;VALUE=DATE:20260924\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n",
        );
        let parsed = parse(source);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        // The same bytes, two readings: the property and VALUE= decide.
        assert_eq!(semantic_of(&parsed, "20260924T100000Z"), "date-time-value");
        assert_eq!(semantic_of(&parsed, "20260924T090000"), "date-time-value");
        assert_eq!(semantic_of(&parsed, "20260924"), "date-value");
        // The syntax layer never claims a reading.
        assert_eq!(crate::lex_host_kind(SyntaxKind::Value), "value");
        assert_eq!(
            crate::lex_host_kind(SyntaxKind::BareParam),
            "bare-param-value"
        );
        assert_eq!(semantic_of(&parsed, "Europe/Berlin"), "bare-param-value");
    }

    #[test]
    fn component_name_bytes_are_read_apart_from_other_values() {
        let opts = HostAnalysisOptions::default();
        let source = "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n";
        let parsed = parse(source);
        assert_eq!(semantic_of(&parsed, "VCALENDAR"), "component-name");
        let tokenization = ENGINE.semantic_tokens(source, &opts).unwrap();
        let kinds: Vec<&str> = names(&tokenization)
            .into_iter()
            .filter(|kind| *kind == "component-name")
            .collect();
        assert_eq!(kinds.len(), 2, "one per BEGIN and END");
        // A value that merely looks like a component name stays a value.
        let parsed = parse(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nBEGIN:VEVENT\r\nUID:1\r\nDTSTAMP:20260924T100000Z\r\nSUMMARY:VTIMEZONE\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        );
        assert_eq!(semantic_of(&parsed, "VTIMEZONE"), "text-value");
        assert_eq!(semantic_of(&parsed, "VEVENT"), "component-name");
        assert_eq!(semantic_of(&parsed, "2.0"), "text-value");
    }

    #[test]
    fn folds_are_marked_and_the_logical_value_still_types() {
        let source = concat!(
            "BEGIN:VCALENDAR\r\n",
            "VERSION:2.0\r\n",
            "PRODID:x\r\n",
            "DTSTART;TZID=Europe/Berlin:202609\r\n",
            " 24T090000\r\n",
            "END:VCALENDAR\r\n",
        );
        let parsed = parse(source);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        let marks: Vec<&str> = parsed
            .lexed()
            .tokens()
            .iter()
            .filter(|token| token.kind == SyntaxKind::FoldMarker)
            .map(|token| token.text(source).unwrap_or_default())
            .collect();
        assert_eq!(marks, vec!["\r\n "]);
        // The two physical value runs are one logical value, and it types.
        let line = parsed
            .lines()
            .iter()
            .find(|line| line.value.len() == 2)
            .expect("folded line");
        assert_eq!(line.value_type, ValueType::DateTime);
        assert_eq!(
            line.value
                .iter()
                .map(|span| span.slice(source).unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["202609", "24T090000"]
        );
        let spans: Vec<Span> = parsed
            .lexed()
            .tokens()
            .iter()
            .map(|token| token.span)
            .collect();
        assert!(
            verify_lossless_spans(source, spans).is_ok(),
            "folding must not move a span"
        );
        // Tab folds count too.
        let tabbed = "DESCRIPTION:one\r\n\ttwo\r\n";
        let parsed = parse(tabbed);
        assert_eq!(
            parsed
                .lexed()
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::FoldMarker)
                .count(),
            1
        );
    }

    #[test]
    fn every_contract_input_is_lossless_at_both_layers() {
        let opts = HostAnalysisOptions::default();
        for source in [
            "",
            "   \t \r\n",
            "\u{FEFF}BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
            "BEGIN:VCALENDAR\nVERSION:2.0\nEND:VCALENDAR\n",
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n",
            "SUMMARY:Ünïcödé — 😀\r\n",
            "DESCRIPTION:folded\r\n value\r\n",
            "ATTACH;FMTTYPE=\"text/plain:https://e.com/a\r\n",
            "BEGIN:VEVENT\r\nSUMMARY:no end here\r\n",
            SAMPLE,
            ":junk\r\n;\r\n\"\r\n",
        ] {
            let parsed = parse(source);
            assert_eq!(parsed.lexed().joined(), source, "{source:?}");
            assert!(
                parsed.lexed().verify_lossless().is_ok(),
                "{source:?} spans did not tile"
            );
            for (layer, tokenization) in [
                ("syntax", ENGINE.lex(source, &opts).unwrap()),
                ("semantic", ENGINE.semantic_tokens(source, &opts).unwrap()),
            ] {
                assert!(host_joins(source, &tokenization), "{source:?} {layer}");
                assert!(
                    verify_lossless_spans(source, host_spans(&tokenization)).is_ok(),
                    "{source:?} {layer}"
                );
                for token in &tokenization.tokens {
                    assert!(
                        token.span.start < token.span.end,
                        "{source:?} {layer} carried a zero-width host span"
                    );
                }
                for diagnostic in &tokenization.diagnostics {
                    assert!(
                        diagnostic.span.start < diagnostic.span.end,
                        "{source:?} {layer} carried a zero-width diagnostic span"
                    );
                }
            }
        }
    }

    #[test]
    fn every_prefix_of_the_sample_parses_and_stays_byte_exact() {
        let opts = HostAnalysisOptions::default();
        for end in 0..SAMPLE.len() {
            if !SAMPLE.is_char_boundary(end) {
                continue;
            }
            let prefix = &SAMPLE[..end];
            let parsed = parse(prefix);
            assert_eq!(parsed.lexed().joined(), prefix, "{end}");
            assert!(
                parsed.lexed().verify_lossless().is_ok(),
                "{end} spans did not tile"
            );
            for token in parsed.lexed().tokens() {
                assert!(
                    !token.span.is_empty() && token.span.is_valid_for(prefix),
                    "{end}"
                );
            }
            for diagnostic in parsed.diagnostics() {
                assert!(!diagnostic.span.is_empty(), "{end} zero-width diagnostic");
                assert!(diagnostic.span.is_valid_for(prefix), "{end} runaway span");
            }
            assert!(ENGINE.lex(prefix, &opts).is_ok(), "{end}");
            assert!(ENGINE.semantic_tokens(prefix, &opts).is_ok(), "{end}");
            assert!(ENGINE.diagnose(prefix, &opts).is_ok(), "{end}");
        }
    }

    #[test]
    fn degenerate_inputs_never_panic_and_always_advance() {
        let opts = HostAnalysisOptions::default();
        for source in [
            ":",
            ";",
            "\"",
            "\\",
            ",",
            "=",
            "X",
            "\r\n",
            "\n",
            "\r",
            "NO COLON HERE\r\n",
            "BEGIN:VEVENT\r\n",
            "END:VEVENT\r\n",
            "BEGIN:\r\n",
            "END:\r\n",
            "BEGIN\r\n",
            "x-Name;PARAM:1\r\n",
            "summary:lowercase name is legal\r\n",
            "DESCRIPTION:folds at eof\r\n ",
            "ATTACH;X=;Y=2:v\r\n",
            "ATTACH;X:v\r\n",
            "ATTACH;X=\"a\":v\r\n",
            "A;B=\"unclosed\r\n",
            "A:B\\",
            "A:B\\:c\r\n",
            "A:B\\\\,C\r\n",
            "BEGIN:VCALENDAR\r\n\u{FEFF}X:1\r\n",
            "DESCRIPTION:😀😀😀 folded\r\n 😀\r\n",
            "\u{FE}",
            "A:B\r\n\t",
            ";;;\r\n",
            "A;B=C,D;E=F:G\r\n",
            "A:B,C\r\n",
            "\r\n \r\n",
            "  \r\n",
        ] {
            let parsed = parse(source);
            assert_eq!(parsed.lexed().joined(), source, "{source:?}");
            assert!(
                parsed.lexed().verify_lossless().is_ok(),
                "{source:?} spans did not tile"
            );
            for span in parsed.lexed().tokens().iter().map(|token| token.span) {
                assert!(!span.is_empty(), "{source:?} emitted a zero-width token");
            }
            for token in parsed.lexed().tokens() {
                assert!(
                    token.text(source).is_some(),
                    "{source:?} un-sliceable token"
                );
            }
            // Every layer answers, and answers the same way.
            let lexed = ENGINE.lex(source, &opts).unwrap();
            let semantic = ENGINE.semantic_tokens(source, &opts).unwrap();
            let diagnostics = ENGINE.diagnose(source, &opts).unwrap();
            assert_eq!(lexed.tokens.len(), semantic.tokens.len(), "{source:?}");
            assert_eq!(lexed.diagnostics, diagnostics, "{source:?}");
            assert_eq!(lexed.valid, parsed.is_valid(), "{source:?}");
        }
    }

    #[test]
    fn pathological_shapes_stay_linear_and_bounded() {
        let deep = "BEGIN:VCALENDAR\r\n".repeat(512);
        let parsed = parse(&deep);
        assert!(parsed.lexed().verify_lossless().is_ok());
        assert!(!parsed.is_valid());
        // Every open component reports its own two faults at two spans.
        let raised: Vec<&str> = parsed
            .diagnostics()
            .iter()
            .copied()
            .map(Diagnostic::code)
            .collect();
        assert_eq!(raised.len(), 1024);
        assert!(
            raised
                .iter()
                .all(|code| *code == "unclosed-component" || *code == "missing-required-property")
        );
        assert_eq!(
            codes(&deep)
                .iter()
                .filter(|code| **code == "unclosed-component")
                .count(),
            512
        );

        let long_line = format!("DESCRIPTION:{}\r\n", "x".repeat(200_000));
        let parsed = parse(&long_line);
        assert!(parsed.lexed().verify_lossless().is_ok());

        let many_folds = format!("DESCRIPTION:v{}\r\n", "\r\n ".repeat(2_000));
        let parsed = parse(&many_folds);
        assert!(parsed.lexed().verify_lossless().is_ok());
        let raised = codes(&many_folds);
        assert!(!raised.contains(&"nothing-to-fold"), "{raised:?}");
        assert!(raised.contains(&"property-before-begin"));
    }

    #[test]
    fn each_diagnostic_code_names_its_own_fault() {
        // Every code the engine can raise is reached by an input of its own.
        let cases: [(&str, &str); 18] = [
            (
                "unclosed-component",
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\n",
            ),
            ("unexpected-end", "END:VCALENDAR\r\n"),
            (
                "component-mismatch",
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            ),
            (
                "missing-vcalendar-wrapper",
                "BEGIN:VEVENT\r\nUID:1\r\nDTSTAMP:20260924T100000Z\r\nEND:VEVENT\r\n",
            ),
            ("property-before-begin", "SUMMARY:no wrapper at all\r\n"),
            ("missing-component-name", "BEGIN:\r\nEND:VCALENDAR\r\n"),
            ("invalid-content-line", "this is not a content line\r\n"),
            (
                "missing-value-delimiter",
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nSUMMARY\r\nEND:VCALENDAR\r\n",
            ),
            (
                "nothing-to-fold",
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\n\r\n dangling fold\r\nEND:VCALENDAR\r\n",
            ),
            (
                "unterminated-quoted-param",
                "ATTACH;FMTTYPE=\"text/plain:https://example.com/a\r\n",
            ),
            ("invalid-escape", "SUMMARY:not\\:an escape\r\n"),
            (
                "invalid-line-ending",
                "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:x\nEND:VCALENDAR\n",
            ),
            (
                "malformed-date",
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nDTSTART;VALUE=DATE:20260931\r\nEND:VCALENDAR\r\n",
            ),
            (
                "malformed-date-time",
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nDTSTART:2026-09-24T09:00\r\nEND:VCALENDAR\r\n",
            ),
            (
                "malformed-duration",
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nDURATION:P1Y\r\nEND:VCALENDAR\r\n",
            ),
            (
                "malformed-period",
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nFREEBUSY:20260924/20260925\r\nEND:VCALENDAR\r\n",
            ),
            (
                "missing-required-property",
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nBEGIN:VEVENT\r\nSUMMARY:no uid\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            ),
            (
                "non-uppercase-name",
                "BEGIN:VCALENDAR\r\nversion:2.0\r\nPRODID:x\r\nEND:VCALENDAR\r\n",
            ),
        ];
        let mut seen: Vec<&str> = Vec::new();
        for (code, source) in cases {
            let raised = codes(source);
            assert!(
                raised.contains(&code),
                "{code} not raised by {source:?}: {raised:?}"
            );
            let diagnostic = validate(source)
                .into_iter()
                .find(|diagnostic| diagnostic.code == code)
                .unwrap_or_else(|| panic!("{code} missing"));
            assert!(!diagnostic.span.is_empty(), "{code} span is zero-width");
            assert!(
                diagnostic.span.is_valid_for(source),
                "{code} span is outside the source"
            );
            assert!(
                source
                    .get(diagnostic.span.range())
                    .is_some_and(|text| !text.is_empty()),
                "{code} span sliced to nothing"
            );
            seen.push(code);
        }
        seen.sort_unstable();
        let expected: Vec<&str> = DiagnosticKind::ALL
            .iter()
            .copied()
            .map(DiagnosticKind::code)
            .collect();
        let mut expected = expected;
        expected.sort_unstable();
        assert_eq!(seen, expected, "every code is covered exactly once");
    }

    #[test]
    fn clean_documents_raise_nothing_and_lowercase_raises_only_a_warning() {
        assert!(codes(SAMPLE).is_empty());
        assert!(codes("").is_empty());
        assert!(codes("\r\n\r\n\r\n").is_empty());
        let source = "BEGIN:VCALENDAR\r\nversion:2.0\r\nPRODID:x\r\nEND:VCALENDAR\r\n";
        let parsed = parse(source);
        assert_eq!(parsed.diagnostics().len(), 1, "{:?}", parsed.diagnostics());
        assert_eq!(parsed.diagnostics()[0].code, "non-uppercase-name");
        assert_eq!(parsed.diagnostics()[0].severity, Severity::Warning);
        // A warning alone keeps the document valid.
        assert!(parsed.is_valid());
        assert!(parsed.errors().next().is_none());
        assert_eq!(parsed.warnings().count(), 1);
        assert!(
            !validate("BEGIN:VCALENDAR\r\nSUMMARY:hi\r\nEND:VEVENT\r\n")
                .iter()
                .all(|diagnostic| diagnostic.severity == Severity::Warning)
        );
        assert_eq!(
            DiagnosticKind::NonUppercaseName.severity(),
            Severity::Warning
        );
        assert_eq!(
            DiagnosticKind::UnclosedComponent.severity(),
            Severity::Error
        );
        assert_eq!(
            DiagnosticKind::MissingRequiredProperty.severity(),
            Severity::Error
        );
    }

    #[test]
    fn required_properties_are_errors_because_the_rfc_says_must() {
        // RFC 5545 §3.6 states these with MUST, so an absent UID is an error,
        // not a stylistic note.
        let source = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nBEGIN:VEVENT\r\nSUMMARY:hi\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let parsed = parse(source);
        assert!(!parsed.is_valid());
        // Two absent properties land on the component's BEGIN marker, and one
        // identical (code, span) pair is never repeated: a second copy of the
        // same diagnostic at the same bytes would say nothing more.
        let raised: Vec<&str> = parsed
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert_eq!(raised, ["missing-required-property"]);
        // Each rule is checked on its own, so one absent property raises it too.
        let only_uid = parse(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nBEGIN:VEVENT\r\nUID:1\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        );
        assert_eq!(only_uid.errors().count(), 1);
        assert_eq!(
            codes("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n"),
            ["missing-required-property"]
        );
        assert!(
            codes("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nEND:VCALENDAR\r\n").is_empty()
        );
        // An unknown component carries no conformance rule, so nothing is imposed.
        assert!(
            codes(
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nBEGIN:X-CUSTOM\r\nEND:X-CUSTOM\r\nEND:VCALENDAR\r\n"
            )
            .is_empty()
        );
        // An unclosed component is still checked for what it required.
        assert!(codes("BEGIN:VEVENT\r\n").contains(&"missing-required-property"));
    }

    #[test]
    fn escapes_are_carved_out_and_only_the_five_rfc_sequences_are_valid() {
        let valid = wrapped("SUMMARY:a\\,b\\;c\\\\d\\nE\\Nf\r\n");
        let parsed = parse(&valid);
        let escapes: Vec<&str> = parsed
            .lexed()
            .tokens()
            .iter()
            .filter(|token| token.kind == SyntaxKind::Escape)
            .map(|token| token.text(&valid).unwrap_or_default())
            .collect();
        assert_eq!(escapes, vec!["\\,", "\\;", "\\\\", "\\n", "\\N"]);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        // The value keeps its bytes: only the escape pairs are carved out.
        assert_eq!(semantic_of(&parsed, "a"), "text-value");

        let bad = wrapped("SUMMARY:not\\:an escape\r\n");
        let parsed = parse(&bad);
        let flagged: Vec<&str> = parsed
            .lexed()
            .tokens()
            .iter()
            .filter(|token| token.has_error())
            .map(|token| crate::lex_host_kind(token.kind))
            .collect();
        assert_eq!(flagged, vec!["escaped-char"], "only the backslash pair");
        assert_eq!(codes(&bad), vec!["invalid-escape"]);
        // A trailing backslash at the end of a line is flagged, not dropped.
        let trailing = wrapped("SUMMARY:ends\\\r\n");
        assert_eq!(codes(&trailing), vec!["invalid-escape"]);
        assert_eq!(parse(&trailing).lexed().joined(), trailing);
        // `\:` is not an RFC 5545 escape, so it is a flagged pair and the bytes
        // after it are still the same value region.
        let colon = wrapped("SUMMARY:a\\:b\r\n");
        let parsed = parse(&colon);
        assert_eq!(codes(&colon), vec!["invalid-escape"]);
        assert_eq!(semantic_of(&parsed, "a"), "text-value");
        assert_eq!(semantic_of(&parsed, "b"), "text-value");
        assert_eq!(
            parsed
                .lexed()
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::Escape)
                .count(),
            1
        );
    }

    #[test]
    fn unterminated_constructs_keep_their_own_span() {
        let source = wrapped("ATTACH;FMTTYPE=\"text/plain:https://example.com/a\r\n");
        let parsed = parse(&source);
        let quoted = parsed
            .lexed()
            .tokens()
            .iter()
            .copied()
            .find(|token| token.kind == SyntaxKind::QuotedParam)
            .expect("quoted run");
        assert!(quoted.has_error());
        assert_eq!(
            quoted.span.slice(&source),
            Some("\"text/plain:https://example.com/a"),
            "the run reaches the end of the physical line"
        );
        assert_eq!(parsed.lexed().joined(), source);
        // The colon was inside the quoted run, so the line never reached a
        // delimiter either: both faults are named, in source order.
        assert_eq!(
            codes(&source),
            vec!["missing-value-delimiter", "unterminated-quoted-param"]
        );
    }

    #[test]
    fn a_param_without_a_value_is_flagged_not_dropped() {
        let source = wrapped("ATTACH;X=;Y=2:v\r\n");
        let parsed = parse(&source);
        assert_eq!(parsed.lexed().joined(), source);
        // `param-value` cannot be empty, and there is no value byte to own the
        // flag, so the `=` carries it.
        let flagged: Vec<&str> = parsed
            .lexed()
            .tokens()
            .iter()
            .filter(|token| token.has_error())
            .map(|token| crate::lex_host_kind(token.kind))
            .collect();
        assert_eq!(flagged, vec!["parameter-assignment"]);
        assert_eq!(codes(&source), vec!["invalid-content-line"]);
        // A parameter with no `=` at all is a flagged run of its own bytes.
        let no_equal = wrapped("ATTACH;X:v\r\n");
        assert_eq!(codes(&no_equal), vec!["invalid-content-line"]);
        assert_eq!(
            parse(&no_equal)
                .lexed()
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::Error)
                .count(),
            1
        );
    }

    #[test]
    fn value_type_for_a_property_comes_from_name_then_param() {
        fn typed(property: &str, param: &str, value: &str) -> (ValueType, Vec<Diagnostic>, String) {
            let source = format!(
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\n{property}{param}:{value}\r\nEND:VCALENDAR\r\n"
            );
            let parsed = parse(&source);
            let token = parsed
                .lexed()
                .tokens()
                .iter()
                .copied()
                .find(|token| token.text(&source) == Some(property))
                .unwrap_or_else(|| panic!("no name token for {property}"));
            let line = parsed
                .lines()
                .iter()
                .find(|line| line.name == Some(token.span))
                .cloned()
                .unwrap_or_else(|| panic!("no line for {property}"));
            let diagnostics = parsed.diagnostics().to_vec();
            (line.value_type, diagnostics, source)
        }
        for (property, param, value, expected) in [
            ("DTSTART", "", "20260924T090000Z", ValueType::DateTime),
            ("DURATION", "", "PT1H", ValueType::Duration),
            ("FREEBUSY", "", "20260924T090000Z/PT1H", ValueType::Period),
            ("RRULE", "", "FREQ=WEEKLY", ValueType::Recurrence),
            ("URL", "", "https://example.com/a", ValueType::Uri),
            ("TRIGGER", "", "-PT15M", ValueType::DurationOrDateTime),
            ("SUMMARY", "", "hello", ValueType::Text),
            ("X-UNKNOWN", "", "whatever", ValueType::Text),
            ("DTSTART", ";VALUE=DATE", "20260924", ValueType::Date),
            (
                "SUMMARY",
                ";VALUE=DATE-TIME",
                "20260924T090000Z",
                ValueType::DateTime,
            ),
            ("SUMMARY", ";VALUE=DURATION", "PT1H", ValueType::Duration),
            (
                "SUMMARY",
                ";VALUE=PERIOD",
                "20260924T090000Z/20260924T100000Z",
                ValueType::Period,
            ),
            (
                "SUMMARY",
                ";VALUE=RECUR",
                "FREQ=DAILY",
                ValueType::Recurrence,
            ),
            (
                "SUMMARY",
                ";VALUE=URI",
                "mailto:a@example.com",
                ValueType::Uri,
            ),
            ("SUMMARY", ";VALUE=TEXT", "a, b; c", ValueType::Text),
            ("SUMMARY", ";VALUE=BOGUS", "hello", ValueType::Text),
        ] {
            let (got, diagnostics, source) = typed(property, param, value);
            assert_eq!(got, expected, "{property}{param} in {source:?}");
            assert!(
                diagnostics.is_empty(),
                "{property}{param}:{value} raised {diagnostics:?}"
            );
        }
    }

    #[test]
    fn structure_is_recorded_without_a_cst() {
        let parsed = parse(SAMPLE);
        let depths: Vec<usize> = parsed.components().iter().map(|c| c.depth).collect();
        // Components are recorded in close order: innermost first.
        assert_eq!(depths, vec![2, 1, 2, 1, 0]);
        assert_eq!(
            parsed
                .components()
                .iter()
                .filter(|component| component.depth == 0)
                .count(),
            1
        );
        assert!(
            parsed
                .components()
                .iter()
                .all(|component| !component.name.is_empty())
        );
        assert!(
            parsed
                .components()
                .iter()
                .all(|component| component.end.is_some())
        );
        assert_eq!(
            parsed
                .lines()
                .iter()
                .filter(|line| line.is_structure)
                .count(),
            10
        );
        assert_eq!(
            parsed
                .lines()
                .iter()
                .filter(|line| !line.is_structure)
                .count(),
            21
        );
        // Every token belongs to exactly one logical line, in order.
        let mut cursor = 0usize;
        for line in parsed.lines() {
            assert_eq!(line.tokens.start, cursor);
            cursor = line.tokens.end;
        }
        assert_eq!(cursor, parsed.lexed().tokens().len());
        // The VEVENT owns the properties between its BEGIN and its END.
        let event = parsed
            .components()
            .iter()
            .find(|component| component.depth == 1 && component.properties.len() > 8)
            .expect("VEVENT properties");
        assert_eq!(event.properties.len(), 12);
    }

    #[test]
    fn host_diagnose_reports_only_icalendar_codes() {
        let opts = HostAnalysisOptions::default();
        let collected = |source: &str| -> Vec<String> {
            ENGINE
                .diagnose(source, &opts)
                .unwrap()
                .iter()
                .map(|diagnostic| diagnostic.code.to_string())
                .collect()
        };
        assert_eq!(collected(SAMPLE), Vec::<String>::new());
        assert_eq!(collected(""), Vec::<String>::new());
        assert_eq!(
            collected("END:VCALENDAR\r\n"),
            vec![
                "unexpected-end".to_string(),
                "missing-vcalendar-wrapper".to_string()
            ]
        );
        assert_eq!(
            collected("BEGIN:VEVENT\r\n"),
            vec![
                "missing-required-property".to_string(),
                "unclosed-component".to_string(),
                "missing-vcalendar-wrapper".to_string(),
            ]
        );
        for code in DiagnosticKind::ALL
            .iter()
            .copied()
            .map(DiagnosticKind::code)
        {
            assert!(
                code.starts_with(|byte: char| byte.is_ascii_lowercase())
                    && code.bytes().all(|byte| byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || byte == b'-')
                    && !code.ends_with('-')
                    && !code.contains("--"),
                "{code} is not kebab-case"
            );
        }
        let mut sorted: Vec<&str> = DiagnosticKind::ALL
            .iter()
            .copied()
            .map(DiagnosticKind::code)
            .collect();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(sorted.len(), before, "codes are unique");
    }

    #[test]
    fn diagnostics_are_ordered_and_deduplicated() {
        let parsed = parse("SUMMARY\r\nSUMMARY\r\n");
        let starts: Vec<usize> = parsed
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.span.start)
            .collect();
        let mut sorted = starts.clone();
        sorted.sort_unstable();
        assert_eq!(starts, sorted, "diagnostics come out in source order");
        // Two required properties missing from one component are two diagnostics
        // at the same span; nothing is reported twice at the same bytes.
        let parsed = parse("BEGIN:VEVENT\r\nSUMMARY:hi\r\nEND:VEVENT\r\n");
        let pairs: Vec<(&str, usize)> = parsed
            .diagnostics()
            .iter()
            .map(|diagnostic| (diagnostic.code, diagnostic.span.start))
            .collect();
        let mut dedup = pairs.clone();
        dedup.dedup();
        assert_eq!(pairs, dedup, "{pairs:?} repeats itself");
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
        assert_eq!(DESCRIPTOR.language, LanguageId::ICS);
        assert_eq!(DESCRIPTOR.display_name, "ics");
        assert_eq!(DESCRIPTOR.extensions, [".ics"]);
        assert_eq!(DESCRIPTOR.mime_types, ["text/calendar"]);
        assert_eq!(DESCRIPTOR.default_dialect, DialectId::DEFAULT);
        assert!(std::ptr::eq(ENGINE.descriptor(), &DESCRIPTOR));
        assert_eq!(ENGINE.id(), LanguageId::ICS);
        assert_eq!(ENGINE.capabilities(), CAPABILITIES);
    }

    #[test]
    fn host_rejects_oversized_input_and_unknown_dialects() {
        let opts = HostAnalysisOptions::default()
            .with_limits(InputLimits::conservative().max_input_bytes(8));
        assert!(matches!(
            ENGINE.lex("BEGIN:VCALENDAR\r\n", &opts),
            Err(HostError::InputTooLarge { max: 8, .. })
        ));
        assert!(matches!(
            ENGINE.semantic_tokens("BEGIN:VCALENDAR\r\n", &opts),
            Err(HostError::InputTooLarge { max: 8, .. })
        ));
        assert!(matches!(
            ENGINE.diagnose("BEGIN:VCALENDAR\r\n", &opts),
            Err(HostError::InputTooLarge { max: 8, .. })
        ));
        assert!(ENGINE.lex("", &opts).is_ok());
        let odd = HostAnalysisOptions::new("rfc6868");
        assert!(matches!(
            ENGINE.lex(SAMPLE, &odd),
            Err(HostError::UnknownDialect { .. })
        ));
    }

    #[test]
    fn error_flags_reach_the_host_wire() {
        let opts = HostAnalysisOptions::default();
        let source = "this is not a line\r\nATTACH;X=\"open:v\r\n";
        let tokenization = ENGINE.lex(source, &opts).unwrap();
        assert!(!tokenization.valid);
        assert!(tokenization.tokens.iter().any(|token| token.error));
        assert!(
            tokenization
                .tokens
                .iter()
                .any(|token| token.kind == "error")
        );
        let semantic = ENGINE.semantic_tokens(source, &opts).unwrap();
        assert_eq!(semantic.tokens.len(), tokenization.tokens.len());
        assert!(
            semantic
                .tokens
                .iter()
                .any(|token| token.kind == "error" && token.error),
            "the semantic layer keeps the flagged reading"
        );
        assert!(host_joins(source, &semantic));
    }

    #[test]
    fn bom_is_its_own_token_and_content_still_reads() {
        let opts = HostAnalysisOptions::default();
        let source = "\u{FEFF}BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nEND:VCALENDAR\r\n";
        let tokenization = ENGINE.lex(source, &opts).unwrap();
        assert!(tokenization.valid, "{:?}", tokenization.diagnostics);
        assert_eq!(tokenization.tokens[0].kind.as_ref(), "bom");
        assert!(host_joins(source, &tokenization));
        // A BOM anywhere else is just unparseable content, and stays byte-exact.
        let inner = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\u{FEFF}\r\nEND:VCALENDAR\r\n";
        let parsed = parse(inner);
        assert_eq!(parsed.lexed().joined(), inner);
        assert!(parsed.lexed().verify_lossless().is_ok());
    }
}
