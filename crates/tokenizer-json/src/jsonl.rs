//! JSON Lines / JSONL: a stream of records, one strict JSON value per line.
//!
//! A `.jsonl` file is not one JSON document, so the JSON parser's "exactly one
//! root value" rule is precisely the wrong rule for it. This module supplies
//! the framing that rule cannot: it splits the document on line terminators,
//! parses every record with the strict JSON parser, and keeps the record
//! boundaries as first-class spans so a host can colour a record break
//! differently from whitespace inside a value.
//!
//! Record text is parsed exactly as JSON, so comments, trailing commas and
//! unquoted keys stay errors here even where JSON and JSON5 allow them; the
//! `application/jsonl` registration says each line is one JSON value.

use crate::{Parse, ParseDiagnostic, ParseOptions, parse_with};
use themoretheless_tokenizer_core::Span;

/// One line of a JSONL document.
#[derive(Debug, Clone, PartialEq)]
pub struct Record<'source> {
    pub index: usize,
    /// Span of the record's own text, excluding its line terminator.
    pub span: Span,
    parsed: Parse<'source>,
}

impl<'source> Record<'source> {
    #[must_use]
    pub fn text(self, source: &'source str) -> Option<&'source str> {
        source.get(self.span.range())
    }

    #[must_use]
    pub const fn parsed(&self) -> &Parse<'source> {
        &self.parsed
    }

    /// This record's diagnostics with spans lifted into document coordinates.
    #[must_use]
    pub fn diagnostics(&self) -> Vec<ParseDiagnostic> {
        self.parsed
            .diagnostics()
            .iter()
            .map(|diagnostic| shift(*diagnostic, self.span.start))
            .collect()
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.parsed.is_valid()
    }
}

/// A parsed JSONL document.
#[derive(Debug, Clone, PartialEq)]
pub struct Jsonl<'source> {
    source: &'source str,
    records: Vec<Record<'source>>,
    line_breaks: Vec<Span>,
    diagnostics: Vec<ParseDiagnostic>,
}

impl<'source> Jsonl<'source> {
    #[must_use]
    pub const fn source(&self) -> &'source str {
        self.source
    }

    #[must_use]
    pub fn records(&self) -> &[Record<'source>] {
        &self.records
    }

    /// Line terminators in document order, including the ones that close a
    /// malformed record.
    #[must_use]
    pub fn line_breaks(&self) -> &[Span] {
        &self.line_breaks
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[ParseDiagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty()
    }

    /// Valid when the document holds at least one record and every record is a
    /// complete JSON value on its own line.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        !self.records.is_empty() && self.diagnostics.is_empty()
    }

    #[must_use]
    pub fn record_at(&self, offset: usize) -> Option<&Record<'source>> {
        self.records
            .iter()
            .find(|record| record.span.start <= offset && offset < record.span.end)
    }
}

/// Parses JSONL with the strict JSON record parser.
#[must_use]
pub fn parse(source: &str) -> Jsonl<'_> {
    parse_records(source, ParseOptions::strict())
}

/// Parses JSONL with explicit record options. A record is always exactly one
/// value, so an option that permits trailing commas is rejected here rather
/// than silently accepted.
#[must_use]
pub fn parse_records(source: &str, options: ParseOptions) -> Jsonl<'_> {
    let records_options = options.allow_trailing_commas(false);
    let mut records = Vec::new();
    let mut line_breaks = Vec::new();
    let mut diagnostics = Vec::new();
    let mut index = 0_usize;
    for (start, end, terminator) in split_lines(source) {
        if let Some(break_span) = terminator {
            line_breaks.push(break_span);
        }
        if start == end {
            continue;
        }
        let parsed = parse_with(&source[start..end], records_options);
        for diagnostic in parsed.diagnostics() {
            diagnostics.push(shift(*diagnostic, start));
        }
        records.push(Record {
            index,
            span: Span::new(start, end),
            parsed,
        });
        index += 1;
    }
    Jsonl {
        source,
        records,
        line_breaks,
        diagnostics,
    }
}

/// Byte ranges of the lines of `source`, each with the terminator that follows
/// it. A document without a trailing terminator ends in one unbroken line.
fn split_lines(source: &str) -> Vec<(usize, usize, Option<Span>)> {
    let bytes = source.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0_usize;
    let mut cursor = 0_usize;
    while cursor < bytes.len() {
        if matches!(bytes[cursor], b'\n' | b'\r') {
            let break_start = cursor;
            cursor = crate::lexer::line_break_end(bytes, cursor);
            lines.push((start, break_start, Some(Span::new(break_start, cursor))));
            start = cursor;
            continue;
        }
        cursor += 1;
    }
    if start < bytes.len() {
        lines.push((start, bytes.len(), None));
    }
    lines
}

fn shift(diagnostic: ParseDiagnostic, offset: usize) -> ParseDiagnostic {
    ParseDiagnostic {
        kind: diagnostic.kind,
        span: Span::new(diagnostic.span.start + offset, diagnostic.span.end + offset),
    }
}
