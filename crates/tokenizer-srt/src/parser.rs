//! Recovering SubRip/WebVTT structure over the lossless token stream.
//!
//! The pass reads tokens only: every diagnostic points at bytes the lexer
//! already emitted, so recovery never consumes, drops or synthesises input.
//! Even the most broken file keeps a lossless token stream while
//! [`Parse::is_valid`] reports `false`, and a warning-level finding — two cues
//! sharing a block, an index that skipped a number, a cue setting WebVTT does
//! not document — never costs a byte either.
//!
//! Structure is deliberately shallow: a cue is the identity line it opens
//! with, its timing line, its settings and its text lines, which is everything
//! a player or an editor needs from timed text. Spans stay raw bytes, so cue
//! text keeps its inline WebVTT tags exactly as written — decoding a
//! `<c.highlight>` span is a reader's job.

use std::fmt;
use std::ops::Range;

use themoretheless_tokenizer_core::{Diagnostic, DiagnosticKind as _, Severity, Span};

use crate::lexer::{LexToken, Lexed, Options, SyntaxKind, lex};

/// A SubRip/WebVTT structural violation with a stable kebab-case code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticKind {
    /// A WebVTT document whose first line is not the `WEBVTT` signature.
    MissingSignature,
    /// Text after `WEBVTT` that is neither empty nor a `- note`.
    UnexpectedHeaderText,
    /// A timestamp that is not the shape the dialect documents.
    MalformedTimestamp,
    /// A timing line whose `-->` is some other run of dashes.
    MissingArrow,
    /// A `-->` that does not sit between exactly two timestamps.
    MissingTimestamp,
    /// A block that should be a cue but never has a timing line.
    MissingTimingLine,
    /// Cue text written between the identity line and the timing line.
    TextBeforeTiming,
    /// A cue that ends before, or exactly when, it starts.
    StartAfterEnd,
    /// A cue that starts before the previous cue has finished.
    OverlappingCues,
    /// A second timing line inside one block, i.e. no blank line between cues.
    MissingBlankLine,
    /// A SubRip index that does not follow the previous index.
    NonMonotonicIndex,
    /// A WebVTT block opening with an all-caps word that is not a marker.
    UnknownBlockType,
    /// A cue setting name WebVTT does not document.
    UnknownCueSetting,
    /// Text after a SubRip timing line, which SubRip has no room for.
    UnexpectedTimingText,
    /// A cue text tag that reaches the end of its line without a `>`.
    UnterminatedMarkup,
}

impl DiagnosticKind {
    /// Every kind this crate can report, ordered by code.
    pub const ALL: [Self; 15] = [
        Self::MalformedTimestamp,
        Self::MissingArrow,
        Self::MissingBlankLine,
        Self::MissingSignature,
        Self::MissingTimingLine,
        Self::MissingTimestamp,
        Self::NonMonotonicIndex,
        Self::OverlappingCues,
        Self::StartAfterEnd,
        Self::TextBeforeTiming,
        Self::UnexpectedHeaderText,
        Self::UnexpectedTimingText,
        Self::UnknownBlockType,
        Self::UnknownCueSetting,
        Self::UnterminatedMarkup,
    ];

    /// Stable wire code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::MissingSignature => "missing-signature",
            Self::UnexpectedHeaderText => "unexpected-header-text",
            Self::MalformedTimestamp => "malformed-timestamp",
            Self::MissingArrow => "missing-arrow",
            Self::MissingTimestamp => "missing-timestamp",
            Self::MissingTimingLine => "missing-timing-line",
            Self::TextBeforeTiming => "text-before-timing",
            Self::StartAfterEnd => "start-after-end",
            Self::OverlappingCues => "overlapping-cues",
            Self::MissingBlankLine => "missing-blank-line",
            Self::NonMonotonicIndex => "non-monotonic-index",
            Self::UnknownBlockType => "unknown-block-type",
            Self::UnknownCueSetting => "unknown-cue-setting",
            Self::UnexpectedTimingText => "unexpected-timing-text",
            Self::UnterminatedMarkup => "unterminated-markup",
        }
    }

    /// Human-readable one-liner.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::MissingSignature => "WebVTT must open with a WEBVTT signature line",
            Self::UnexpectedHeaderText => "text after WEBVTT must be a note after a hyphen",
            Self::MalformedTimestamp => "timestamp does not match this format's grammar",
            Self::MissingArrow => "timing line has no `-->` arrow",
            Self::MissingTimestamp => "timing arrow needs exactly two timestamps",
            Self::MissingTimingLine => "cue block has no timing line",
            Self::TextBeforeTiming => "cue text appears before the timing line",
            Self::StartAfterEnd => "cue does not end after it starts",
            Self::OverlappingCues => "cue starts before the previous cue ends",
            Self::MissingBlankLine => "cues must be separated by an empty line",
            Self::NonMonotonicIndex => "cue index does not follow the previous index",
            Self::UnknownBlockType => "block type is not one WebVTT defines",
            Self::UnknownCueSetting => "cue setting is not one WebVTT defines",
            Self::UnexpectedTimingText => "SubRip timing lines carry no settings",
            Self::UnterminatedMarkup => "cue text tag is never closed",
        }
    }

    /// Warnings describe legal-but-suspect documents; errors mean bytes a
    /// reader cannot turn into a cue.
    #[must_use]
    pub const fn severity(self) -> Severity {
        match self {
            Self::MissingSignature
            | Self::UnexpectedHeaderText
            | Self::MalformedTimestamp
            | Self::MissingArrow
            | Self::MissingTimestamp
            | Self::MissingTimingLine
            | Self::TextBeforeTiming
            | Self::StartAfterEnd
            | Self::UnterminatedMarkup => Severity::Error,
            Self::OverlappingCues
            | Self::MissingBlankLine
            | Self::NonMonotonicIndex
            | Self::UnknownBlockType
            | Self::UnknownCueSetting
            | Self::UnexpectedTimingText => Severity::Warning,
        }
    }

    /// The kind behind a wire code, so a host can recover severity from a bare
    /// [`Diagnostic`].
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.code() == code)
    }

    /// Whether only SubRip can report this kind.
    #[must_use]
    pub const fn only_srt(self) -> bool {
        matches!(self, Self::NonMonotonicIndex | Self::UnexpectedTimingText)
    }

    /// Whether only WebVTT can report this kind.
    #[must_use]
    pub const fn only_vtt(self) -> bool {
        matches!(
            self,
            Self::MissingSignature
                | Self::UnexpectedHeaderText
                | Self::UnknownBlockType
                | Self::UnknownCueSetting
                | Self::UnterminatedMarkup
        )
    }
}

impl themoretheless_tokenizer_core::DiagnosticKind for DiagnosticKind {
    fn code(self) -> &'static str {
        DiagnosticKind::code(self)
    }

    fn message(self) -> &'static str {
        DiagnosticKind::message(self)
    }

    fn severity(self) -> Severity {
        DiagnosticKind::severity(self)
    }
}

impl fmt::Display for DiagnosticKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

/// One timestamp of a cue's timing line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    /// The whole timestamp, every field of it, as written.
    pub span: Span,
    /// Its value in milliseconds, with the fractional part read at its own
    /// scale, so `.5`, `.50` and `.500` all mean 500 ms.
    pub millis: u64,
}

/// One `name:value` cue setting after a timing line's end timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setting {
    /// The name, its `:` and its value, as written.
    pub span: Span,
    /// Where the name is.
    pub name: Span,
    /// Where the value is, or `None` for a setting written without a value.
    pub value: Option<Span>,
    /// Whether WebVTT documents this name.
    pub known: bool,
}

/// One `Name: value` line of a WebVTT file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderField {
    /// The whole line, as written.
    pub span: Span,
    /// Where the name is.
    pub name: Span,
    /// Where the value is, or `None` for `Style:` written empty.
    pub value: Option<Span>,
}

/// One cue: the identity line it opens with, its timing line, and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    /// From the first byte of the cue's first line to the last byte of its
    /// last text line, so a cue covers what a player would show.
    pub span: Span,
    /// The identity line's bytes, when the cue has one.
    pub identity: Option<Span>,
    /// The timing line, without its record break.
    pub timing: Span,
    /// The `-->` arrow.
    pub arrow: Option<Span>,
    /// The start timestamp.
    pub start: Option<Timestamp>,
    /// The end timestamp.
    pub end: Option<Timestamp>,
    /// Settings after the end timestamp: WebVTT's, or SubRip's excess text.
    pub settings: Vec<Setting>,
    /// Cue text lines, in order, without their record breaks.
    pub text: Vec<Span>,
}

impl Cue {
    /// Whether the cue's timing line was recovered.
    #[must_use]
    pub const fn has_timing(&self) -> bool {
        self.arrow.is_some()
    }
}

/// Lossless tokens plus SubRip/WebVTT diagnostics and a flat cue table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse<'source> {
    lexed: Lexed<'source>,
    diagnostics: Vec<Diagnostic>,
    cues: Vec<Cue>,
    headers: Vec<HeaderField>,
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

    /// Cues in document order, including ones recovered from a malformed
    /// timing line.
    #[must_use]
    pub fn cues(&self) -> &[Cue] {
        &self.cues
    }

    /// The `Style:` and `Region:` lines of a WebVTT file header.
    #[must_use]
    pub fn headers(&self) -> &[HeaderField] {
        &self.headers
    }

    /// The bytes one [`Cue`] field points at.
    #[must_use]
    pub fn text(&self, span: Span) -> &'source str {
        self.lexed.source().get(span.range()).unwrap_or_default()
    }

    /// Whether the document is structurally clean. An error-flagged token is
    /// always paired with an error-severity diagnostic, so both must agree;
    /// warnings leave a document valid, because its bytes still read exactly
    /// as written.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        !self.lexed.has_errors()
            && self.diagnostics.iter().all(|diagnostic| {
                DiagnosticKind::from_code(diagnostic.code)
                    .is_none_or(|kind| kind.severity() != Severity::Error)
            })
    }

    #[must_use]
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }
}

/// Lex `source` and recover as much cue structure as its bytes allow.
#[must_use]
pub fn parse(source: &str, options: Options) -> Parse<'_> {
    let lexed = lex(source, options);
    let mut diagnostics = Vec::new();
    let (cues, headers) = {
        let doc = Doc {
            source,
            tokens: lexed.tokens(),
            lines: line_ranges(lexed.tokens()),
        };
        doc.flagged_diagnostics(options, &mut diagnostics);
        doc.read_structure(options, &mut diagnostics)
    };
    diagnostics.sort_by_key(|diagnostic| (diagnostic.span.start, diagnostic.code));
    Parse {
        lexed,
        diagnostics,
        cues,
        headers,
    }
}

/// Structural diagnostics for a document, the check a host calls.
#[must_use]
pub fn validate(source: &str, options: Options) -> Vec<Diagnostic> {
    parse(source, options).into_diagnostics()
}

/// Whether a line of tokens is a cue's timing line.
pub(crate) fn is_timing_line(line: &[LexToken]) -> bool {
    line.iter()
        .any(|token| token.is_timing_field() || token.kind == SyntaxKind::TimingArrow)
}

/// The physical lines of a token stream, each ending at its record break.
pub(crate) fn line_ranges(tokens: &[LexToken]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut start = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        if token.kind == SyntaxKind::RecordBreak {
            out.push(start..index + 1);
            start = index + 1;
        }
    }
    if start < tokens.len() {
        out.push(start..tokens.len());
    }
    out
}

/// One token stream read as lines and blocks. `tokens` may borrow for shorter
/// than `source`, because every span it hands back is a [`Span`], not a
/// reference into the token vector.
struct Doc<'source, 'tokens> {
    source: &'source str,
    tokens: &'tokens [LexToken],
    lines: Vec<Range<usize>>,
}

impl<'source, 'tokens> Doc<'source, 'tokens> {
    fn line(&self, line: usize) -> Range<usize> {
        self.lines.get(line).cloned().unwrap_or(0..0)
    }

    fn token(&self, index: usize) -> LexToken {
        self.tokens[index]
    }

    fn is_blank(&self, line: usize) -> bool {
        self.line(line)
            .clone()
            .all(|index| self.token(index).kind.is_trivia())
    }

    /// The line's content, with its record break left out.
    fn line_span(&self, line: usize) -> Span {
        let range = self.line(line);
        let Some(first) = range.clone().find(|&i| !self.token(i).kind.is_trivia()) else {
            return range
                .map(|i| self.token(i).span)
                .next()
                .unwrap_or(Span::new(0, 0));
        };
        let last = range
            .rev()
            .find(|&i| self.token(i).kind != SyntaxKind::RecordBreak)
            .unwrap_or(first);
        Span::new(self.token(first).span.start, self.token(last).span.end)
    }

    fn spans(&self, from_line: usize, to_line: usize) -> Span {
        self.line_span(from_line).cover(self.line_span(to_line))
    }

    /// Blocks: maximal runs of non-blank lines, by line index.
    fn blocks(&self) -> Vec<Vec<usize>> {
        let mut out: Vec<Vec<usize>> = Vec::new();
        let mut current: Vec<usize> = Vec::new();
        for line in 0..self.lines.len() {
            if self.is_blank(line) {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            } else {
                current.push(line);
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
        out
    }

    /// A line opening where a timestamp or arrow must, which is what makes a
    /// cue text line that starts with a digit a malformed timing line.
    fn is_timing_line(&self, line: usize) -> bool {
        is_timing_line(&self.tokens[self.line(line)])
    }

    fn has_kind(&self, line: usize, kind: SyntaxKind) -> bool {
        self.line(line).any(|index| self.token(index).kind == kind)
    }

    /// The token kinds of a line's significant tokens.
    fn significant(&self, line: usize) -> Vec<LexToken> {
        self.line(line)
            .map(|index| self.token(index))
            .filter(|token| !token.kind.is_trivia())
            .collect()
    }

    /// Diagnostics for the spans the lexer already flagged: one per malformed
    /// timestamp group, per broken arrow, per unclosed tag, per setting the
    /// dialect does not document.
    fn flagged_diagnostics(&self, options: Options, out: &mut Vec<Diagnostic>) {
        let mut setting_reports: std::collections::HashSet<usize> =
            std::collections::HashSet::new();
        for range in &self.lines {
            for group in self.timestamp_groups(range.clone()) {
                if group.clone().any(|index| self.token(index).has_bad_time()) {
                    let span = Span::new(
                        self.token(group.start).span.start,
                        self.token(group.end - 1).span.end,
                    );
                    push(out, DiagnosticKind::MalformedTimestamp, span);
                }
            }
            for index in range.clone() {
                let token = self.token(index);
                if token.is_bad_arrow() {
                    push(out, DiagnosticKind::MissingArrow, token.span);
                }
                if token.is_unclosed() && token.kind == SyntaxKind::MarkupPunctuation {
                    push(out, DiagnosticKind::UnterminatedMarkup, token.span);
                }
                if token.kind == SyntaxKind::SettingName {
                    if options.cue_settings {
                        if token.is_unknown_setting() {
                            push(out, DiagnosticKind::UnknownCueSetting, token.span);
                        }
                    } else if setting_reports.insert(range.start) {
                        // One finding per timing line, not per excess word.
                        push(out, DiagnosticKind::UnexpectedTimingText, token.span);
                    }
                }
            }
        }
    }

    /// Maximal runs of timestamp fields, each starting at a head field.
    fn timestamp_groups(&self, range: Range<usize>) -> Vec<Range<usize>> {
        let mut out: Vec<Range<usize>> = Vec::new();
        let mut index = range.start;
        while index < range.end {
            let token = self.token(index);
            if token.is_timing_field() && token.is_timestamp_head() {
                let start = index;
                index += 1;
                while index < range.end && self.token(index).is_timing_field() {
                    index += 1;
                }
                out.push(start..index);
            } else {
                index += 1;
            }
        }
        out
    }

    /// The well-formed timestamps of a line, in order.
    fn timestamps(&self, line: usize) -> Vec<Timestamp> {
        let mut out = Vec::new();
        for group in self.timestamp_groups(self.line(line)) {
            if group.clone().any(|index| self.token(index).has_bad_time()) {
                continue;
            }
            let Some(millis) = self.millis_of(group.clone()) else {
                continue;
            };
            out.push(Timestamp {
                span: Span::new(
                    self.token(group.start).span.start,
                    self.token(group.end - 1).span.end,
                ),
                millis,
            });
        }
        out
    }

    /// The value a timestamp's fields state, or `None` when a field is missing.
    fn millis_of(&self, group: Range<usize>) -> Option<u64> {
        let mut hour: Option<u64> = None;
        let mut minute: Option<u64> = None;
        let mut second: Option<u64> = None;
        let mut fraction: Option<(u64, usize)> = None;
        for index in group {
            let token = self.token(index);
            if !matches!(
                token.kind,
                SyntaxKind::TimeHour
                    | SyntaxKind::TimeMinute
                    | SyntaxKind::TimeSecond
                    | SyntaxKind::Millisecond
            ) {
                continue;
            }
            let digits = token.text(self.source)?;
            let value = digits.parse::<u64>().ok()?;
            match token.kind {
                SyntaxKind::TimeHour => hour = Some(value),
                SyntaxKind::TimeMinute => minute = Some(value),
                SyntaxKind::TimeSecond => second = Some(value),
                _ => fraction = Some((value, digits.len())),
            }
        }
        let (value, width) = fraction.unwrap_or((0, 3));
        let scaled = match width {
            1 => value.saturating_mul(100),
            2 => value.saturating_mul(10),
            _ => value,
        };
        let minutes = minute?;
        let seconds = second?;
        Some(
            hour.unwrap_or(0)
                .saturating_mul(3_600_000)
                .saturating_add(minutes.saturating_mul(60_000))
                .saturating_add(seconds.saturating_mul(1_000))
                .saturating_add(scaled),
        )
    }

    /// The document's one structural pass: the header block, then every cue.
    fn read_structure(
        &self,
        options: Options,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> (Vec<Cue>, Vec<HeaderField>) {
        let mut cues: Vec<Cue> = Vec::new();
        let mut headers: Vec<HeaderField> = Vec::new();
        let first_line = (0..self.lines.len()).find(|&line| !self.is_blank(line));
        if options.programmatic_header {
            let signed = first_line.is_some_and(|line| self.has_kind(line, SyntaxKind::Signature));
            if let Some(line) = first_line.filter(|_| !signed) {
                push(
                    diagnostics,
                    DiagnosticKind::MissingSignature,
                    self.line_span(line),
                );
            }
        }
        let mut state = Order {
            previous_end: None,
            previous_index: None,
        };
        for block in self.blocks() {
            let Some(&opening) = block.first() else {
                continue;
            };
            // Under WebVTT the block holding the signature line is the file
            // header, up to the first timing line; everything after it is a cue.
            let header_lines = if options.programmatic_header && Some(opening) == first_line {
                block
                    .iter()
                    .position(|&line| self.is_timing_line(line))
                    .unwrap_or(block.len())
            } else {
                0
            };
            if header_lines > 0 {
                self.read_header(&block[..header_lines], &mut headers, diagnostics);
            }
            let rest: Vec<usize> = block[header_lines..].to_vec();
            let timings: Vec<usize> = rest
                .iter()
                .enumerate()
                .filter(|&(_, &line)| self.is_timing_line(line))
                .map(|(position, _)| position)
                .collect();
            if timings.is_empty() {
                if let Some(&first) = rest.first() {
                    self.read_untimed_block(first, options, diagnostics);
                }
                continue;
            }
            for (ordinal, &position) in timings.iter().enumerate() {
                if ordinal > 0 {
                    // A second timing line in one block: the blank line that
                    // should have separated two cues is what is missing.
                    push(
                        diagnostics,
                        DiagnosticKind::MissingBlankLine,
                        self.line_span(rest[position]),
                    );
                }
                let from = if ordinal == 0 {
                    0
                } else {
                    timings[ordinal - 1] + 1
                };
                let to = timings.get(ordinal + 1).copied().unwrap_or(rest.len());
                let before: &[usize] = if ordinal == 0 {
                    &rest[from..position]
                } else {
                    &[]
                };
                self.read_cue(
                    before,
                    rest[position],
                    &rest[position + 1..to],
                    options,
                    &mut cues,
                    &mut state,
                    diagnostics,
                );
            }
        }
        (cues, headers)
    }

    /// A block with no timing line at all: a comment, style or region block
    /// passes quietly, an all-caps word names a block type WebVTT does not
    /// have, and anything else is a cue nobody finished writing.
    fn read_untimed_block(&self, first: usize, options: Options, out: &mut Vec<Diagnostic>) {
        if self.has_kind(first, SyntaxKind::BlockMarker) {
            return;
        }
        if options.programmatic_header && self.is_block_keyword(first) {
            push(out, DiagnosticKind::UnknownBlockType, self.line_span(first));
            return;
        }
        push(
            out,
            DiagnosticKind::MissingTimingLine,
            self.line_span(first),
        );
    }

    /// Whether a line is a single all-caps word, the shape of a block marker
    /// that WebVTT does not define.
    fn is_block_keyword(&self, line: usize) -> bool {
        let tokens = self.significant(line);
        match tokens.as_slice() {
            [only] => {
                let text = only.text(self.source).unwrap_or_default();
                text.len() >= 2
                    && text.len() <= 32
                    && text.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte == b'_' || byte.is_ascii_digit()
                    })
            }
            _ => false,
        }
    }

    /// The `WEBVTT` line and the `Name: value` fields under it.
    fn read_header(
        &self,
        lines: &[usize],
        headers: &mut Vec<HeaderField>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let Some(&signature) = lines.first() else {
            return;
        };
        for index in self.line(signature) {
            let token = self.token(index);
            if token.kind == SyntaxKind::Error {
                push(
                    diagnostics,
                    DiagnosticKind::UnexpectedHeaderText,
                    token.span,
                );
            }
        }
        for &line in &lines[1..] {
            let tokens = self.significant(line);
            let name = tokens.iter().find(|t| t.kind == SyntaxKind::HeaderName);
            let separator = tokens
                .iter()
                .find(|t| t.kind == SyntaxKind::HeaderSeparator);
            let value = tokens.iter().find(|t| t.kind == SyntaxKind::HeaderValue);
            if let (Some(name), Some(_)) = (name, separator) {
                headers.push(HeaderField {
                    span: self.line_span(line),
                    name: name.span,
                    value: value.map(|value| value.span),
                });
            }
        }
    }

    /// One cue: the identity line before its timing line, the timing line
    /// itself, and the text lines below it.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn read_cue(
        &self,
        before: &[usize],
        timing: usize,
        text: &[usize],
        options: Options,
        cues: &mut Vec<Cue>,
        state: &mut Order,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let identity = before
            .last()
            .copied()
            .filter(|&line| self.is_identity_line(line));
        // One fault per cue: the first stray line names the whole misplaced run.
        let stray = &before[..before.len() - usize::from(identity.is_some())];
        if let Some(&line) = stray.first() {
            push(
                diagnostics,
                DiagnosticKind::TextBeforeTiming,
                self.line_span(line),
            );
        }
        let tokens = self.significant(timing);
        let arrow = tokens
            .iter()
            .find(|token| token.kind == SyntaxKind::TimingArrow)
            .map(|token| token.span);
        let broken = tokens
            .iter()
            .any(|token| token.has_bad_time() || token.is_bad_arrow());
        let stamps = self.timestamps(timing);
        let mut cue = Cue {
            span: self.spans(
                before.first().copied().unwrap_or(timing),
                *text.last().unwrap_or(&timing),
            ),
            identity: identity.map(|line| self.line_span(line)),
            timing: self.line_span(timing),
            arrow,
            start: None,
            end: None,
            settings: self.settings_of(timing),
            text: text
                .iter()
                .copied()
                .map(|line| self.line_span(line))
                .collect(),
        };
        match (arrow, broken, stamps.as_slice()) {
            (Some(_), false, [start, end]) => {
                cue.start = Some(*start);
                cue.end = Some(*end);
                self.check_times(*start, *end, state, diagnostics);
            }
            (Some(_), false, _) => {
                push(
                    diagnostics,
                    DiagnosticKind::MissingTimestamp,
                    self.line_span(timing),
                );
            }
            // A line with no arrow, or one already flagged, was named by
            // `flagged_diagnostics`: `missing-arrow` or `malformed-timestamp`.
            _ => {}
        }
        if let Some(span) = cue.identity {
            self.check_index(span, options, state, diagnostics);
        }
        cues.push(cue);
    }

    /// Whether a line is a cue's identity line: one token the lexer settled on
    /// an identity kind.
    fn is_identity_line(&self, line: usize) -> bool {
        matches!(
            self.significant(line).as_slice(),
            [only] if only.kind.is_identity()
        )
    }

    /// Start before end, and not inside the previous cue.
    fn check_times(
        &self,
        start: Timestamp,
        end: Timestamp,
        state: &mut Order,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if start.millis > end.millis {
            push(
                diagnostics,
                DiagnosticKind::StartAfterEnd,
                start.span.cover(end.span),
            );
        }
        if state
            .previous_end
            .is_some_and(|previous| start.millis < previous)
        {
            push(diagnostics, DiagnosticKind::OverlappingCues, start.span);
        }
        state.previous_end = Some(end.millis.max(start.millis));
    }

    /// SubRip numbers its cues, and a reader notices when the numbers stop
    /// rising.
    fn check_index(
        &self,
        span: Span,
        options: Options,
        state: &mut Order,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let text = span.slice(self.source).unwrap_or_default().trim();
        let Ok(value) = text.parse::<u64>() else {
            return;
        };
        if !options.ordinal_index {
            state.previous_index = Some(value);
            return;
        }
        if state
            .previous_index
            .is_some_and(|previous| value <= previous)
        {
            push(diagnostics, DiagnosticKind::NonMonotonicIndex, span);
        }
        state.previous_index = Some(value);
    }

    /// The `name:value` pairs after a timing line's end timestamp.
    fn settings_of(&self, line: usize) -> Vec<Setting> {
        let tokens = self.significant(line);
        let mut out: Vec<Setting> = Vec::new();
        let mut index = 0usize;
        while index < tokens.len() {
            if tokens[index].kind != SyntaxKind::SettingName {
                index += 1;
                continue;
            }
            let name = tokens[index];
            let separator = (index + 1 < tokens.len()
                && tokens[index + 1].kind == SyntaxKind::SettingSeparator)
                .then(|| tokens[index + 1]);
            let value = separator.and_then(|_| {
                (index + 2 < tokens.len() && tokens[index + 2].kind == SyntaxKind::SettingValue)
                    .then(|| tokens[index + 2])
            });
            let last = value.or(separator).map_or(name.span, |token| token.span);
            out.push(Setting {
                span: name.span.cover(last),
                name: name.span,
                value: value.map(|value| value.span),
                known: !name.is_unknown_setting(),
            });
            index += 3;
        }
        out
    }
}

/// Order tracking across cues.
struct Order {
    previous_end: Option<u64>,
    previous_index: Option<u64>,
}

fn push(out: &mut Vec<Diagnostic>, kind: DiagnosticKind, span: Span) {
    out.push(kind.to_diagnostic(span));
}
