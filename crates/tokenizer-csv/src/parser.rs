//! Recovering CSV/TSV structure over the lossless token stream.
//!
//! The pass reads tokens only: every diagnostic points at bytes the lexer
//! already emitted, so recovery never consumes or drops input. Even the most
//! broken file keeps a lossless token stream while [`Parse::is_valid`]
//! reports `false`.
//!
//! Structure is deliberately shallow — records and their fields, not a tree:
//! a record's identity is its field count and its span, which is everything
//! an editor or data tool needs from this format.

use std::fmt;
use std::ops::Range;

use themoretheless_tokenizer_core::{Diagnostic, DiagnosticKind as _, Span};

use crate::lexer::{LexToken, Lexed, Options, SyntaxKind, lex};

/// A CSV/TSV structural violation with a stable kebab-case code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticKind {
    /// A quoted field hit EOF (or ran out of input) without a closing quote.
    UnclosedQuote,
    /// Content sits between a closing `"` and the next delimiter or break.
    TextAfterClosingQuote,
    /// A record's field count differs from the header record's.
    RaggedRow,
}

impl DiagnosticKind {
    /// Stable wire code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnclosedQuote => "unclosed-quote",
            Self::TextAfterClosingQuote => "text-after-closing-quote",
            Self::RaggedRow => "ragged-row",
        }
    }

    /// Human-readable one-liner.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnclosedQuote => "quoted field is never closed",
            Self::TextAfterClosingQuote => "text between a closing quote and the field's end",
            Self::RaggedRow => "record has a different field count than the header",
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

/// One parsed field. A zero-length field carries no span — the lossless
/// stream never contains a zero-width token, and neither does the structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    /// The field's bytes including its surrounding quotes, or `None` when the
    /// field is zero-length.
    pub span: Option<Span>,
    /// Whether the field was written as a quoted CSV field.
    pub quoted: bool,
    /// Whether the field region contains a flagged (malformed) span.
    pub has_error: bool,
}

impl Field {
    /// The field's raw bytes, quotes included; `None` for a zero-length
    /// field.
    #[must_use]
    pub fn text<'source>(&self, source: &'source str) -> Option<&'source str> {
        self.span.and_then(|span| span.slice(source))
    }
}

/// One parsed record. The span covers the record's bytes *and* its trailing
/// record break, so even a blank line owns a non-empty span to report on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub span: Span,
    /// Index range into [`Parse::fields`].
    pub fields: Range<usize>,
    /// Whether this is the first record, whose field count the others must
    /// match.
    pub is_header: bool,
}

/// Lossless tokens plus CSV/TSV diagnostics and a flat record/field table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse<'source> {
    lexed: Lexed<'source>,
    diagnostics: Vec<Diagnostic>,
    records: Vec<Record>,
    fields: Vec<Field>,
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
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// The fields belonging to one record.
    #[must_use]
    pub fn record_fields(&self, record: &Record) -> &[Field] {
        self.fields.get(record.fields.clone()).unwrap_or_default()
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
pub fn parse(source: &str, options: Options) -> Parse<'_> {
    let lexed = lex(source, options);
    let mut diagnostics = Vec::new();
    flagged_diagnostics(&lexed, &mut diagnostics);
    let (records, fields) = read_structure(&lexed);
    ragged_diagnostics(&records, &mut diagnostics);
    diagnostics.sort_by_key(|diagnostic| diagnostic.span.start);
    Parse {
        lexed,
        diagnostics,
        records,
        fields,
    }
}

/// Structural diagnostics only.
#[must_use]
pub fn validate(source: &str, options: Options) -> Vec<Diagnostic> {
    parse(source, options).into_diagnostics()
}

/// Diagnostics for the spans the lexer already flagged.
fn flagged_diagnostics(lexed: &Lexed<'_>, output: &mut Vec<Diagnostic>) {
    for token in lexed.tokens() {
        match token.kind {
            // An unclosed quoted field flags its opening quote; the abandoned
            // content keeps its field kind so the stream stays classifiable.
            SyntaxKind::Quote if token.has_error() => {
                output.push(DiagnosticKind::UnclosedQuote.to_diagnostic(token.span));
            }
            SyntaxKind::Error => {
                output.push(DiagnosticKind::TextAfterClosingQuote.to_diagnostic(token.span));
            }
            _ => {}
        }
    }
}

/// The field being accumulated between two delimiters or breaks.
#[derive(Debug, Default)]
struct FieldAccum {
    span: Option<Span>,
    quoted: bool,
    has_error: bool,
}

impl FieldAccum {
    fn add(&mut self, token: LexToken) {
        self.span = Some(self.span.map_or(token.span, |span| span.cover(token.span)));
        self.quoted |=
            token.is_quoted() || matches!(token.kind, SyntaxKind::Quote | SyntaxKind::EscapedQuote);
        self.has_error |= token.has_error();
    }

    fn close_into(&mut self, out: &mut Vec<Field>) {
        out.push(Field {
            span: self.span,
            quoted: self.quoted,
            has_error: self.has_error,
        });
        *self = Self::default();
    }
}

/// Splits the token stream into records and their fields. Quoted regions
/// never contain `delimiter` or `record-break` tokens (the lexer keeps those
/// bytes as field text), so this walk needs no quoting state of its own.
fn read_structure(lexed: &Lexed<'_>) -> (Vec<Record>, Vec<Field>) {
    let mut records = Vec::new();
    let mut fields: Vec<Field> = Vec::new();
    let mut field = FieldAccum::default();
    let mut record_begin = 0usize;
    let mut record_fields_start = 0usize;
    let mut has_content = false;
    let mut content_end = 0usize;

    for token in lexed.tokens() {
        match token.kind {
            SyntaxKind::Bom => record_begin = token.span.end,
            SyntaxKind::Delimiter => {
                field.close_into(&mut fields);
                has_content = true;
                content_end = token.span.end;
            }
            SyntaxKind::RecordBreak => {
                field.close_into(&mut fields);
                records.push(Record {
                    span: Span::new(record_begin, token.span.end),
                    fields: record_fields_start..fields.len(),
                    is_header: records.is_empty(),
                });
                record_fields_start = fields.len();
                record_begin = token.span.end;
                has_content = false;
            }
            SyntaxKind::HeaderField
            | SyntaxKind::Field
            | SyntaxKind::Quote
            | SyntaxKind::EscapedQuote
            | SyntaxKind::Error => {
                field.add(*token);
                has_content = true;
                content_end = token.span.end;
            }
        }
    }
    if has_content {
        field.close_into(&mut fields);
        records.push(Record {
            span: Span::new(record_begin, content_end),
            fields: record_fields_start..fields.len(),
            is_header: records.is_empty(),
        });
    }
    (records, fields)
}

/// Report every record whose field count differs from the header's.
fn ragged_diagnostics(records: &[Record], output: &mut Vec<Diagnostic>) {
    let Some(header) = records.first() else {
        return;
    };
    let expected = header.fields.len().max(1);
    for record in records.iter().skip(1) {
        if record.fields.len() != expected {
            output.push(DiagnosticKind::RaggedRow.to_diagnostic(record.span));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(source: &str, options: Options) -> Vec<&'static str> {
        validate(source, options)
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect()
    }

    #[test]
    fn valid_documents_have_no_diagnostics() {
        for (source, options) in [
            ("name,score\nada,42\ngrace,7\n", Options::CSV),
            ("a,\"b,c\",\"d\"\"e\"\n1,2,3\n", Options::CSV),
            ("a,\"multi\r\nline\",b\nx,y,z\n", Options::CSV),
            ("\u{FEFF}h1,h2\nv1,v2\n", Options::CSV),
            ("name\tscore\nada\t42\n", Options::TSV),
            ("a,b\tc\nd,e\tf\n", Options::TSV),
            ("", Options::CSV),
            ("\n", Options::CSV),
            ("a\n", Options::TSV),
        ] {
            let parsed = parse(source, options);
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
    fn records_and_fields_cover_every_field() {
        let parsed = parse("h1,h2,h3\nplain,,\"quoted, row\"\n", Options::CSV);
        assert!(parsed.is_valid());
        let records = parsed.records();
        assert_eq!(records.len(), 2);
        assert!(records[0].is_header);
        assert!(!records[1].is_header);
        assert_eq!(records[0].span, Span::new(0, 9));
        assert_eq!(records[1].span, Span::new(9, 30));
        let header: Vec<Option<&str>> = parsed
            .record_fields(&records[0])
            .iter()
            .map(|field| field.text(parsed.lexed().source()))
            .collect();
        assert_eq!(header, vec![Some("h1"), Some("h2"), Some("h3")]);
        let body = parsed.record_fields(&records[1]);
        assert_eq!(body.len(), 3);
        assert_eq!(body[1].span, None, "a zero-length field carries no span");
        assert!(!body[1].quoted);
        assert!(body[2].quoted);
        assert_eq!(
            body[2].text(parsed.lexed().source()),
            Some("\"quoted, row\"")
        );
    }

    #[test]
    fn record_spans_include_their_breaks() {
        let parsed = parse("a\r\nb\n", Options::CSV);
        let records = parsed.records();
        assert_eq!(records[0].span, Span::new(0, 3));
        assert_eq!(records[1].span, Span::new(3, 5));
        let blank = parse("\n\n", Options::CSV);
        assert_eq!(blank.records().len(), 2);
        assert_eq!(blank.records()[0].span, Span::new(0, 1));
        assert!(blank.is_valid(), "two blank rows agree on one field each");
    }

    #[test]
    fn unclosed_quote_is_reported_and_recovered() {
        let source = "a,b\nc,\"oops";
        assert_eq!(codes(source, Options::CSV), vec!["unclosed-quote"]);
        let parsed = parse(source, Options::CSV);
        assert!(!parsed.is_valid());
        assert!(parsed.lexed().is_lossless());
        assert_eq!(parsed.lexed().joined(), source);
    }

    #[test]
    fn text_after_closing_quote_is_reported() {
        assert_eq!(
            codes("a,\"x\"y,b\n", Options::CSV),
            vec!["text-after-closing-quote"]
        );
        let parsed = parse("a,\"x\"y,b\n", Options::CSV);
        assert!(!parsed.is_valid());
        assert!(parsed.lexed().is_lossless());
    }

    #[test]
    fn ragged_rows_are_reported_against_the_header() {
        assert_eq!(codes("h1,h2\none\n", Options::CSV), vec!["ragged-row"]);
        assert_eq!(
            codes("h1,h2\na,b\nc,d,e\nf\ng,h\n", Options::CSV),
            vec!["ragged-row", "ragged-row"]
        );
        assert_eq!(codes("a\tb\nc\n", Options::TSV), vec!["ragged-row"]);
        let parsed = parse("h1,h2\none\n", Options::CSV);
        assert!(!parsed.is_valid());
        assert!(parsed.lexed().is_lossless());
        assert_eq!(
            parsed.diagnostics()[0].span,
            Span::new(6, 10),
            "reported on the offending record"
        );
    }

    #[test]
    fn a_headerless_single_record_cannot_be_ragged() {
        assert_eq!(codes("only,one,row\n", Options::CSV), Vec::<&str>::new());
        assert_eq!(codes("a,b,c", Options::TSV), Vec::<&str>::new());
    }

    #[test]
    fn tsv_has_no_quote_diagnostics_at_all() {
        // Quotes are field text in TSV, so nothing can be unclosed.
        let source = "a\tb\nq,\"x\ty\n";
        assert_eq!(codes(source, Options::TSV), Vec::<&str>::new());
        let parsed = parse(source, Options::TSV);
        assert!(
            !parsed.lexed().tokens().iter().any(|token| {
                matches!(token.kind, SyntaxKind::Quote | SyntaxKind::EscapedQuote)
            })
        );
        assert_eq!(parsed.fields()[2].text(source), Some("q,\"x"));
    }

    #[test]
    fn recovery_keeps_the_stream_lossless() {
        let broken = concat!(
            "\u{FEFF}h1,h2\n",
            "ragged\n",
            "\"unclosed row\n",
            "still,more\"junk,x\n",
            "final,record",
        );
        let parsed = parse(broken, Options::CSV);
        assert!(parsed.lexed().is_lossless());
        assert_eq!(parsed.lexed().joined(), broken);
        assert!(!parsed.is_valid());
        assert!(!parsed.diagnostics().is_empty());
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
        let source = "h,h\n\"a\"x,y\nshort\nunclosed,\"oops\n";
        let diagnostics = validate(source, Options::CSV);
        assert!(
            diagnostics
                .windows(2)
                .all(|pair| pair[0].span.start <= pair[1].span.start),
            "{diagnostics:?}"
        );
        let codes: Vec<&str> = diagnostics.iter().map(|d| d.code).collect();
        assert!(codes.contains(&"unclosed-quote"), "{codes:?}");
        assert!(codes.contains(&"text-after-closing-quote"), "{codes:?}");
    }

    #[test]
    fn every_truncation_still_produces_consistent_structure() {
        let sample = "h1,h2\n\"a,b\",3\nx\ny,\"z\"\"q\"w,\"tail";
        for options in [Options::CSV, Options::TSV] {
            for cut in 0..=sample.len() {
                let source = &sample[..cut];
                let parsed = parse(source, options);
                assert!(parsed.lexed().is_lossless(), "{source:?}");
                let total: usize = parsed
                    .records()
                    .iter()
                    .map(|record| parsed.record_fields(record).len())
                    .sum();
                assert_eq!(total, parsed.fields().len(), "{source:?}");
                for record in parsed.records() {
                    assert!(!record.span.is_empty(), "{source:?}");
                }
                for field in parsed.fields() {
                    assert!(
                        field.span.is_none_or(|span| !span.is_empty()),
                        "{source:?} has a zero-width field span"
                    );
                }
            }
        }
    }
}
