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
//! `application/jsonl` registration says each line is one JSON value. That
//! guarantee holds for [`parse_records`] as well, which adopts only the
//! resource limits a caller supplies, never its grammar.
//!
//! A line that carries no text at all is skipped rather than parsed, so leading,
//! embedded and trailing blank lines produce neither a record nor a diagnostic.
//! This is the usual tolerance of the format; the line terminator itself stays
//! visible as a record break.

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

/// Strict JSON grammar carrying over only the caller's resource limits.
///
/// `LexerOptions::json5_mode(false)` clears the JSON5 grammar along with the
/// comments and byte-order marks it implies, so the three knobs that widen the
/// grammar are all forced off; `max_input_bytes`, `max_tokens`,
/// `max_diagnostics` and `max_depth` are left exactly as supplied.
fn strict_records(options: ParseOptions) -> ParseOptions {
    options
        .json5_mode(false)
        .allow_comments(false)
        .allow_trailing_commas(false)
}

/// Parses JSONL with the strict JSON record parser.
#[must_use]
pub fn parse(source: &str) -> Jsonl<'_> {
    parse_records(source, ParseOptions::strict())
}

/// Parses JSONL with explicit record options. A record is always one strict
/// JSON value, so only the resource limits a caller supplies are adopted here;
/// the grammar is forced back to strict JSON, rejecting trailing commas,
/// comments, single-quoted strings and unquoted keys even where JSON5 and
/// JSONC accept them.
#[must_use]
pub fn parse_records(source: &str, options: ParseOptions) -> Jsonl<'_> {
    let records_options = strict_records(options);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json5_options_cannot_widen_the_record_grammar() {
        for (construct, source) in [
            ("unquoted key", "{a:1}\n"),
            ("single-quoted string", "{'a':1}\n"),
            ("line comment", "//x\n{\"a\":1}\n"),
            ("trailing comma", "{\"a\":[1,]}\n"),
        ] {
            assert!(
                !parse_records(source, ParseOptions::json5()).is_valid(),
                "{construct} was accepted through JSON5 options"
            );
        }
        assert!(parse_records("{\"a\":1}\n", ParseOptions::json5()).is_valid());
    }

    #[test]
    fn caller_resource_limits_survive_the_strict_forcing() {
        let source = "{\"a\":true}\n";
        assert!(
            parse_records(source, ParseOptions::strict().max_input_bytes(4)).has_errors(),
            "strict grammar forcing reset the caller's input limit"
        );
        assert!(parse_records(source, ParseOptions::strict()).is_valid());
    }

    #[test]
    fn blank_lines_are_skipped_but_their_breaks_stay_visible() {
        let document = parse("\n{\"a\":1}\n");
        assert_eq!(document.records().len(), 1);
        assert!(document.diagnostics().is_empty());
        assert_eq!(document.line_breaks().len(), 2);
    }
}
