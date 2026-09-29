//! Recovering logfmt structure over the lossless token stream.
//!
//! The pass reads tokens only: every diagnostic points at bytes the lexer
//! already emitted, so recovery never consumes or drops input. Even the most
//! broken document keeps a lossless token stream while [`Parse::is_valid`]
//! reports `false`.
//!
//! Structure is deliberately shallow — records and their pairs, not a tree: a
//! record's identity is its pair list and its span, which is everything an
//! editor or a log viewer needs from this format. Values are kept as raw
//! source regions, quotes and escapes included; unescaping is a consumer's
//! job, because an engine that invents decoded bytes can no longer prove that
//! the spans it hands back were ever in the input.

use std::fmt;
use std::ops::Range;

use themoretheless_tokenizer_core::{Diagnostic, DiagnosticKind as _, Span};

use crate::lexer::{LexToken, Lexed, SyntaxKind, lex};

/// A logfmt structural violation with a stable kebab-case code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticKind {
    /// A `"` opened a value that never reached its closing quote.
    UnterminatedValue,
    /// An `=` appeared with no key in front of it.
    MissingKey,
    /// A bare run is welded directly onto a complete pair's closing quote.
    UnexpectedToken,
}

impl DiagnosticKind {
    /// Stable wire code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnterminatedValue => "unterminated-value",
            Self::MissingKey => "missing-key",
            Self::UnexpectedToken => "unexpected-token",
        }
    }

    /// Human-readable one-liner.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnterminatedValue => "quoted value is never closed",
            Self::MissingKey => "separator has no key before it",
            Self::UnexpectedToken => "text follows a closing quote with no space",
        }
    }
}

impl themoretheless_tokenizer_core::DiagnosticKind for DiagnosticKind {
    fn code(self) -> &'static str {
        DiagnosticKind::code(self)
    }

    fn message(self) -> &'static str {
        DiagnosticKind::message(self)
    }
}

impl fmt::Display for DiagnosticKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

/// One parsed value, kept exactly as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Value {
    /// The value's bytes: the surrounding quotes are included, so the span is
    /// always a region of the source itself.
    pub span: Span,
    /// Whether the value was written between double quotes. A quoted value is
    /// text by construction and therefore never takes a numeric or boolean
    /// reading in the semantic layer.
    pub quoted: bool,
    /// Whether the value region contains a flagged (malformed) span.
    pub has_error: bool,
}

impl Value {
    /// The value's raw bytes, quotes and escapes included.
    #[must_use]
    pub fn text<'source>(&self, source: &'source str) -> Option<&'source str> {
        self.span.slice(source)
    }
}

/// One parsed pair: a key plus, unless it is a flag, a separator and whatever
/// value followed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pair {
    /// Key through end of value; a flag pair spans just its key.
    pub span: Span,
    /// The key's bytes.
    pub key: Span,
    /// The `=` byte, or `None` for a stand-alone flag key.
    pub separator: Option<Span>,
    /// The value, or `None` for `key=` (an empty value) and for a flag pair.
    pub value: Option<Value>,
}

impl Pair {
    /// Whether the pair asserts a key's presence without naming a value.
    #[must_use]
    pub const fn is_flag(&self) -> bool {
        self.separator.is_none()
    }

    /// Whether the pair was written as `key=` with no value bytes at all.
    #[must_use]
    pub const fn is_empty_value(&self) -> bool {
        self.separator.is_some() && self.value.is_none()
    }

    /// The key's bytes.
    #[must_use]
    pub fn key_text<'source>(&self, source: &'source str) -> Option<&'source str> {
        self.key.slice(source)
    }

    /// The value's raw bytes; `None` for a flag or an empty value.
    #[must_use]
    pub fn value_text<'source>(&self, source: &'source str) -> Option<&'source str> {
        self.value.and_then(|value| value.text(source))
    }
}

/// One parsed record. The span covers the record's bytes *and* its trailing
/// record break, so even a blank line owns a non-empty span to report on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub span: Span,
    /// Index range into [`Parse::pairs`].
    pub pairs: Range<usize>,
}

impl Record {
    /// Whether the record holds no pairs: a blank or whitespace-only line.
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.pairs.is_empty()
    }
}

/// Lossless tokens plus logfmt diagnostics and a flat record/pair table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse<'source> {
    lexed: Lexed<'source>,
    diagnostics: Vec<Diagnostic>,
    records: Vec<Record>,
    pairs: Vec<Pair>,
    empty_value_separators: Vec<Span>,
}

impl<'source> Parse<'source> {
    #[must_use]
    pub const fn lexed(&self) -> &Lexed<'source> {
        &self.lexed
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    #[must_use]
    pub fn pairs(&self) -> &[Pair] {
        &self.pairs
    }

    /// The pairs belonging to one record.
    #[must_use]
    pub fn record_pairs(&self, record: &Record) -> &[Pair] {
        self.pairs.get(record.pairs.clone()).unwrap_or_default()
    }

    /// Spans of the `=` bytes that introduce an empty value, in source order.
    #[must_use]
    pub fn empty_value_separators(&self) -> &[Span] {
        &self.empty_value_separators
    }

    /// Whether this separator byte introduces a value-less pair (`msg=`).
    ///
    /// A value with no bytes has no span of its own, and a lossless stream may
    /// not carry a zero-width token, so the `=` carries the empty reading
    /// instead: the syntax layer names it `separator`, the semantic layer
    /// re-tags exactly this span as `empty-value`.
    #[must_use]
    pub fn is_empty_value(&self, separator: Span) -> bool {
        self.empty_value_separators
            .binary_search_by_key(&separator.start, |span| span.start)
            .is_ok()
    }

    /// Whether the document is structurally clean. An error-flagged token is
    /// always paired with a diagnostic, so both must be empty.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.diagnostics.is_empty() && !self.lexed.has_errors()
    }

    #[must_use]
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }
}

/// Runs the lossless lexer and the recovering structural pass.
#[must_use]
pub fn parse(source: &str) -> Parse<'_> {
    let lexed = lex(source);
    let mut diagnostics = Vec::new();
    flagged_diagnostics(&lexed, &mut diagnostics);
    let (records, pairs) = read_structure(&lexed);
    let empty_value_separators: Vec<Span> = pairs
        .iter()
        .filter(|pair| pair.is_empty_value())
        .filter_map(|pair| pair.separator)
        .collect();
    diagnostics.sort_by_key(|diagnostic| diagnostic.span.start);
    Parse {
        lexed,
        diagnostics,
        records,
        pairs,
        empty_value_separators,
    }
}

/// Structural diagnostics only.
#[must_use]
pub fn validate(source: &str) -> Vec<Diagnostic> {
    parse(source).into_diagnostics()
}

/// Diagnostics for the spans the lexer already flagged.
fn flagged_diagnostics(lexed: &Lexed<'_>, output: &mut Vec<Diagnostic>) {
    for token in lexed.tokens() {
        if !token.has_error() {
            continue;
        }
        let kind = match token.kind {
            // An unterminated quoted value flags its opening run; the bytes it
            // swallowed keep their value kind so the stream stays classifiable.
            SyntaxKind::QuotedValue => DiagnosticKind::UnterminatedValue,
            // A flagged separator is the only way an `=` without a key survives.
            SyntaxKind::Separator => DiagnosticKind::MissingKey,
            _ => DiagnosticKind::UnexpectedToken,
        };
        output.push(kind.to_diagnostic(token.span));
    }
}

/// The pair being accumulated between two keys or record breaks.
#[derive(Debug, Default)]
struct PairAccum {
    key: Option<Span>,
    separator: Option<Span>,
    value: Option<Value>,
}

impl PairAccum {
    /// Emits the accumulated pair. A run holding no key emits nothing: a stray
    /// `=` is a diagnostic, not a pair.
    fn close_into(&mut self, out: &mut Vec<Pair>) {
        let Some(key) = self.key else {
            *self = Self::default();
            return;
        };
        let end = self
            .value
            .map_or_else(|| self.separator.unwrap_or(key), |value| value.span)
            .end;
        out.push(Pair {
            span: Span::new(key.start, end),
            key,
            separator: self.separator,
            value: self.value,
        });
        *self = Self::default();
    }

    /// Extends the pending value with one more of its runs. A value's runs are
    /// contiguous in the source, so covering them yields the exact region.
    fn add_value(&mut self, token: LexToken) {
        let quoted = matches!(
            token.kind,
            SyntaxKind::QuotedValue | SyntaxKind::EscapedChar
        );
        match self.value {
            Some(value) => {
                let span = Span::new(value.span.start, token.span.end);
                self.value = Some(Value {
                    span,
                    quoted: value.quoted || quoted,
                    has_error: value.has_error || token.has_error(),
                });
            }
            None => {
                self.value = Some(Value {
                    span: token.span,
                    quoted,
                    has_error: token.has_error(),
                });
            }
        }
    }
}

/// Splits the token stream into records and their pairs. Quoted regions never
/// contain `whitespace` or `record-break` tokens (the lexer keeps those bytes
/// as value text), so this walk needs no quoting state of its own.
fn read_structure(lexed: &Lexed<'_>) -> (Vec<Record>, Vec<Pair>) {
    let mut records = Vec::new();
    let mut pairs: Vec<Pair> = Vec::new();
    let mut pair = PairAccum::default();
    let mut record_begin = 0usize;
    let mut record_pairs_start = 0usize;
    let mut has_content = false;
    let mut content_end = 0usize;

    for token in lexed.tokens() {
        match token.kind {
            SyntaxKind::Bom => record_begin = token.span.end,
            SyntaxKind::Whitespace => {
                has_content = true;
                content_end = token.span.end;
            }
            SyntaxKind::RecordBreak => {
                pair.close_into(&mut pairs);
                records.push(Record {
                    span: Span::new(record_begin, token.span.end),
                    pairs: record_pairs_start..pairs.len(),
                });
                record_pairs_start = pairs.len();
                record_begin = token.span.end;
                has_content = false;
            }
            SyntaxKind::Key | SyntaxKind::FlagKey => {
                pair.close_into(&mut pairs);
                pair.key = Some(token.span);
                has_content = true;
                content_end = token.span.end;
            }
            SyntaxKind::Separator => {
                if !token.has_error() {
                    pair.separator = Some(token.span);
                }
                has_content = true;
                content_end = token.span.end;
            }
            SyntaxKind::BareValue | SyntaxKind::QuotedValue | SyntaxKind::EscapedChar => {
                pair.add_value(*token);
                has_content = true;
                content_end = token.span.end;
            }
            // Stray text welded onto a closing quote ends the pair: it is a
            // diagnostic, never a slice of the value's bytes.
            SyntaxKind::Error => {
                pair.close_into(&mut pairs);
                has_content = true;
                content_end = token.span.end;
            }
        }
    }
    pair.close_into(&mut pairs);
    if has_content {
        records.push(Record {
            span: Span::new(record_begin, content_end),
            pairs: record_pairs_start..pairs.len(),
        });
    }
    (records, pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(source: &str) -> Vec<&'static str> {
        validate(source).iter().map(|d| d.code).collect()
    }

    /// `(key, raw value)` for every pair, values quoted exactly as written.
    fn pair_texts<'a>(parsed: &Parse<'a>) -> Vec<(&'a str, Option<&'a str>)> {
        let source = parsed.lexed().source();
        parsed
            .pairs()
            .iter()
            .map(|pair| {
                (
                    pair.key_text(source).unwrap_or_default(),
                    pair.value_text(source),
                )
            })
            .collect()
    }

    #[test]
    fn valid_documents_have_no_diagnostics() {
        for source in [
            "level=info msg=\"starting server\" addr=0.0.0.0:8080\n",
            "msg=\"say \\\"hi\\\"\" path=\"C:\\\\tmp\"\n",
            "a=1 b=2 c=3\nd=4\n",
            "msg=\"multi\nline\" tail=1\n",
            "-v --dry-run\n",
            "msg=\n",
            "a=b=c\n",
            "a=\"x\" b=\"y\"\n",
            "trailing=break\r\n",
            "\u{FEFF}level=warn msg=\"bom\"\n",
            "名前=値 emoji=😀\n",
            "",
            "\n",
            "\n\n",
            "   \n",
            "msg=x",
            "msg=x   ",
        ] {
            let parsed = parse(source);
            assert!(
                parsed.diagnostics().is_empty(),
                "{source:?} {:?}",
                parsed.diagnostics()
            );
            assert!(parsed.is_valid(), "{source:?}");
            assert!(parsed.lexed().is_lossless(), "{source:?}");
        }
    }

    #[test]
    fn records_and_pairs_cover_every_pair() {
        let source = "a=1 b=2\r\nc=3 d=\"four five\"\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let records = parsed.records();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].span, Span::new(0, 9));
        assert_eq!(records[1].span, Span::new(9, 27));
        assert_eq!(parsed.record_pairs(&records[0]).len(), 2);
        assert_eq!(parsed.record_pairs(&records[1]).len(), 2);
        assert_eq!(
            pair_texts(&parsed),
            vec![
                ("a", Some("1")),
                ("b", Some("2")),
                ("c", Some("3")),
                ("d", Some("\"four five\"")),
            ]
        );
    }

    #[test]
    fn a_pair_needs_no_break_to_be_a_record() {
        let parsed = parse("a=1");
        assert_eq!(parsed.records().len(), 1);
        assert_eq!(parsed.records()[0].span, Span::new(0, 3));
        assert_eq!(
            parse("a=1\n").records()[0].span,
            Span::new(0, 4),
            "break in"
        );
        assert_eq!(parse("").records().len(), 0);
        assert_eq!(parse("").pairs().len(), 0);
    }

    #[test]
    fn record_spans_include_their_breaks() {
        let parsed = parse("a=1\r\nb=2\n");
        let records = parsed.records();
        assert_eq!(records[0].span, Span::new(0, 5));
        assert_eq!(records[1].span, Span::new(5, 9));
        let blank = parse("\n\n");
        assert_eq!(blank.records().len(), 2);
        assert_eq!(blank.records()[0].span, Span::new(0, 1));
        assert!(blank.records()[0].is_blank());
        assert!(blank.is_valid(), "blank records are ordinary logfmt");
    }

    #[test]
    fn flag_pairs_carry_no_separator_or_value() {
        let source = "-v --dry-run msg=x\n";
        let parsed = parse(source);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        let pairs = parsed.pairs();
        assert_eq!(pairs.len(), 3);
        assert!(pairs[0].is_flag());
        assert_eq!(pairs[0].separator, None);
        assert_eq!(pairs[0].value, None);
        assert_eq!(pairs[0].span, Span::new(0, 2));
        assert!(!pairs[2].is_flag());
        assert!(!pairs[2].is_empty_value());
        assert_eq!(pairs[2].value_text(source), Some("x"));
    }

    #[test]
    fn empty_values_are_pairs_with_no_value_bytes() {
        let source = "msg= tail=1\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let pairs = parsed.pairs();
        assert_eq!(pairs.len(), 2);
        assert!(pairs[0].is_empty_value());
        assert!(!pairs[1].is_empty_value());
        assert_eq!(pairs[0].span, Span::new(0, 4), "the pair ends at its =");
        assert_eq!(pairs[0].value, None);
        assert_eq!(
            parsed.empty_value_separators(),
            &[Span::new(3, 4)],
            "exactly one empty-value separator"
        );
        assert!(parsed.is_empty_value(Span::new(3, 4)));
        assert!(!parsed.is_empty_value(Span::new(9, 10)));
        assert!(parse("msg=").is_empty_value(Span::new(3, 4)));
    }

    #[test]
    fn quoted_empty_value_is_not_an_empty_value() {
        let source = "msg=\"\"\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        assert!(parsed.pairs()[0].value.is_some());
        assert!(parsed.empty_value_separators().is_empty());
        assert_eq!(parsed.pairs()[0].value_text(source), Some("\"\""));
    }

    #[test]
    fn unterminated_value_is_reported_and_recovered() {
        let source = "a=1\nmsg=\"oops";
        assert_eq!(codes(source), vec!["unterminated-value"]);
        let parsed = parse(source);
        assert!(!parsed.is_valid());
        assert!(parsed.lexed().is_lossless());
        assert_eq!(parsed.lexed().joined(), source);
        assert_eq!(parsed.pairs().len(), 2, "the broken pair is still a pair");
        assert!(parsed.pairs()[1].value.is_some());
        assert!(parsed.pairs()[1].value.unwrap().has_error);
        assert_eq!(parsed.records().len(), 2);
    }

    #[test]
    fn an_unterminated_value_swallows_the_rest_of_the_document() {
        let source = "a=1 msg=\"oops\nb=2\n";
        assert_eq!(codes(source), vec!["unterminated-value"]);
        let parsed = parse(source);
        assert_eq!(parsed.records().len(), 1, "the break became value text");
        assert_eq!(parsed.pairs().len(), 2);
        assert_eq!(parsed.pairs()[1].value_text(source), Some("\"oops\nb=2\n"));
    }

    #[test]
    fn missing_key_is_reported_and_holds_no_pair() {
        let source = "msg=x =oops\n";
        assert_eq!(codes(source), vec!["missing-key"]);
        let parsed = parse(source);
        assert!(!parsed.is_valid());
        assert_eq!(
            pair_texts(&parsed),
            vec![("msg", Some("x")), ("oops", None)],
            "the orphan `=` joins no pair, and its own key is not invented"
        );
        assert_eq!(parsed.records().len(), 1);
        assert!(parsed.lexed().is_lossless());
        assert_eq!(codes("=oops\n"), vec!["missing-key"]);
        assert_eq!(codes("a=1\n\n=2\n"), vec!["missing-key"]);
        assert_eq!(codes("msg = hi\n"), vec!["missing-key"]);
    }

    #[test]
    fn unexpected_token_is_reported_for_glued_text() {
        let source = "msg=\"hi\"there\n";
        assert_eq!(codes(source), vec!["unexpected-token"]);
        let parsed = parse(source);
        assert!(!parsed.is_valid());
        assert!(parsed.lexed().is_lossless());
        assert_eq!(
            parsed.diagnostics()[0].span,
            Span::new(8, 13),
            "reported on the glued run, not on the value"
        );
        assert_eq!(
            parsed.pairs()[0].value_text(source),
            Some("\"hi\""),
            "the glued run is a diagnostic, not value bytes"
        );
        assert_eq!(parsed.pairs()[0].span, Span::new(0, 8));
        assert_eq!(codes("a=1 b=\"x\"y=c d=2\n"), vec!["unexpected-token"]);
    }

    #[test]
    fn a_space_after_a_closing_quote_is_no_incident() {
        assert_eq!(codes("a=1 b=\"x\" c=2\n"), Vec::<&str>::new());
        assert_eq!(codes("a=\"x\"\nb=\"y\"\n"), Vec::<&str>::new());
        assert_eq!(codes("a=\"x\""), Vec::<&str>::new());
        assert_eq!(codes("a=\"x\"\t\n"), Vec::<&str>::new());
    }

    #[test]
    fn values_keep_their_quotes_and_escapes_verbatim() {
        let source = "msg=\"say \\\"hi\\\"\"\n";
        let parsed = parse(source);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        let quoted = parsed.pairs()[0].value.unwrap();
        assert!(quoted.quoted, "a quoted value says so");
        assert_eq!(quoted.span, Span::new(4, 16));
        assert_eq!(quoted.text(source), Some("\"say \\\"hi\\\"\""));
        let bare = parse("a=1\n").pairs()[0].value.unwrap();
        assert!(!bare.quoted);
        assert_eq!(bare.text("a=1\n"), Some("1"));
    }

    #[test]
    fn continuation_records_span_their_quoted_newlines() {
        let source = "a=1 msg=\"one\ntwo\" b=2\nc=3\n";
        let parsed = parse(source);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        let records = parsed.records();
        assert_eq!(records.len(), 2);
        assert_eq!(
            parsed.record_pairs(&records[0]).len(),
            3,
            "the line inside the quotes did not open a record"
        );
        assert_eq!(records[0].span, Span::new(0, 22));
        assert_eq!(records[1].span, Span::new(22, 26));
    }

    #[test]
    fn recovery_keeps_the_stream_lossless() {
        let broken = concat!(
            "\u{FEFF}a=1\n",
            "msg=\"oops\"glued\n",
            "=orphan tail=x\n",
            "final=record",
        );
        let parsed = parse(broken);
        assert!(parsed.lexed().is_lossless());
        assert_eq!(parsed.lexed().joined(), broken);
        assert!(!parsed.is_valid());
        let observed: Vec<&str> = parsed
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert_eq!(observed, vec!["unexpected-token", "missing-key"]);
        for diagnostic in parsed.diagnostics() {
            assert!(!diagnostic.span.is_empty(), "{diagnostic:?} is zero-width");
            assert!(
                diagnostic.span.is_valid_for(broken),
                "{diagnostic:?} escapes the source"
            );
        }
    }

    #[test]
    fn diagnostics_are_in_source_order() {
        let source = "=a b=\"x\"glued\nmsg=\"oops\nc=1\n";
        let diagnostics = validate(source);
        assert!(
            diagnostics
                .windows(2)
                .all(|pair| pair[0].span.start <= pair[1].span.start),
            "{diagnostics:?}"
        );
        let observed: Vec<&str> = diagnostics.iter().map(|d| d.code).collect();
        assert_eq!(
            observed,
            vec!["missing-key", "unexpected-token", "unterminated-value"]
        );
    }

    #[test]
    fn every_diagnostic_kind_has_a_stable_code_and_message() {
        let all = [
            DiagnosticKind::UnterminatedValue,
            DiagnosticKind::MissingKey,
            DiagnosticKind::UnexpectedToken,
        ];
        let wire: Vec<&str> = all.iter().map(|kind| kind.code()).collect();
        assert_eq!(
            wire,
            vec!["unterminated-value", "missing-key", "unexpected-token"]
        );
        for kind in all {
            assert!(!kind.message().is_empty());
            assert_eq!(
                kind.to_diagnostic(Span::new(0, 1)).code,
                kind.code(),
                "the core trait and the inherent code agree"
            );
            assert_eq!(format!("{kind}"), kind.message());
        }
    }

    #[test]
    fn blank_lines_and_leading_whitespace_stay_clean() {
        let parsed = parse("\n   msg=x\n\n");
        assert!(parsed.is_valid());
        assert_eq!(parsed.records().len(), 3);
        assert!(parsed.records()[0].is_blank());
        assert!(!parsed.records()[1].is_blank());
        assert!(parsed.records()[2].is_blank());
        assert_eq!(parsed.pairs().len(), 1);
        assert_eq!(parsed.pairs()[0].key, Span::new(4, 7));
        assert_eq!(parsed.records()[1].span, Span::new(1, 10));
    }

    #[test]
    fn structure_never_invents_or_forgets_a_key() {
        let source = "a=1 b=2\n\nx=\"y\" -f z=\n";
        let parsed = parse(source);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        assert_eq!(
            pair_texts(&parsed),
            vec![
                ("a", Some("1")),
                ("b", Some("2")),
                ("x", Some("\"y\"")),
                ("-f", None),
                ("z", None),
            ]
        );
        for pair in parsed.pairs() {
            assert!(
                source
                    .get(pair.key.range())
                    .is_some_and(|text| !text.is_empty()),
                "{pair:?} has no key bytes"
            );
        }
    }

    #[test]
    fn every_truncation_still_produces_consistent_structure() {
        let sample =
            "a=1 msg=\"two \"\"words\"\r\nlevel=info -v x=\"un\nclosed\"tail=\"\n=orphan b=\n";
        for cut in 0..=sample.len() {
            let source = &sample[..cut];
            let parsed = parse(source);
            assert!(parsed.lexed().is_lossless(), "{source:?}");
            assert_eq!(parsed.lexed().joined(), source, "{source:?}");
            let total: usize = parsed
                .records()
                .iter()
                .map(|record| parsed.record_pairs(record).len())
                .sum();
            assert_eq!(total, parsed.pairs().len(), "{source:?}");
            for record in parsed.records() {
                assert!(!record.span.is_empty(), "{source:?}");
            }
            for pair in parsed.pairs() {
                assert!(!pair.span.is_empty(), "{source:?}");
                assert!(!pair.key.is_empty(), "{source:?}");
                assert!(pair.span.contains(pair.key.start), "{source:?}");
                if let Some(separator) = pair.separator {
                    assert_eq!(separator.len(), 1, "{source:?}");
                    assert_eq!(source.as_bytes()[separator.start], b'=', "{source:?}");
                    if let Some(value) = pair.value {
                        assert_eq!(separator.end, value.span.start, "{source:?}");
                    }
                }
            }
        }
    }
}
