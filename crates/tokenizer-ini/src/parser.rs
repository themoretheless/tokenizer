//! Recovering INI/properties structure over the lossless token stream.
//!
//! The pass reads tokens only: every diagnostic points at bytes the lexer
//! already emitted, so recovery never consumes or drops input. Even the most
//! broken file keeps a lossless token stream while [`Parse::is_valid`] reports
//! `false`, and a warning-level finding — a duplicate section, a key before
//! any `[section]`, an unknown `.properties` escape — never drops bytes either.
//!
//! Structure is deliberately shallow — sections and their entries, not a tree:
//! an entry's identity is its spans, which is everything an editor or config
//! tool needs from this format. Spans are raw bytes: a value keeps its quotes,
//! escapes and continuation breaks exactly as written, because decoding
//! escapes is a reader's job and would invent bytes this pass never claims to
//! own.

use std::collections::HashMap;
use std::fmt;
use std::ops::Range;

use themoretheless_tokenizer_core::{Diagnostic, DiagnosticKind as _, Severity, Span};

use crate::lexer::{LexToken, Lexed, Options, SyntaxKind, lex};

/// An INI/properties structural violation with a stable kebab-case code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticKind {
    /// A `[section` header reached its record break without a closing `]`.
    UnterminatedSection,
    /// A section name is declared by a second header.
    DuplicateSection,
    /// A key appears before any section header, which INI discourages.
    KeyOutsideSection,
    /// A quoted key or value reached its record break without a closing `"`.
    UnclosedQuote,
    /// Content sits between a closing `"` and the end of the line.
    TextAfterClosingQuote,
    /// Content sits between a closing `]` and the end of the header line.
    TextAfterSectionHeader,
    /// A backslash introduces something `.properties` does not document.
    InvalidEscape,
}

impl DiagnosticKind {
    /// Every kind this crate can report.
    pub const ALL: [Self; 7] = [
        Self::UnterminatedSection,
        Self::DuplicateSection,
        Self::KeyOutsideSection,
        Self::UnclosedQuote,
        Self::TextAfterClosingQuote,
        Self::TextAfterSectionHeader,
        Self::InvalidEscape,
    ];

    /// Stable wire code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnterminatedSection => "unterminated-section",
            Self::DuplicateSection => "duplicate-section",
            Self::KeyOutsideSection => "key-outside-section",
            Self::UnclosedQuote => "unclosed-quote",
            Self::TextAfterClosingQuote => "text-after-closing-quote",
            Self::TextAfterSectionHeader => "text-after-section-header",
            Self::InvalidEscape => "invalid-escape",
        }
    }

    /// Human-readable one-liner.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnterminatedSection => "section header is never closed",
            Self::DuplicateSection => "section name is declared twice",
            Self::KeyOutsideSection => "key appears outside any section",
            Self::UnclosedQuote => "quoted key or value is never closed",
            Self::TextAfterClosingQuote => "text between a closing quote and the line's end",
            Self::TextAfterSectionHeader => "text between a closing bracket and the line's end",
            Self::InvalidEscape => "escape sequence is not one properties documents",
        }
    }

    /// Warnings describe legal-but-odd documents; errors mean bytes the writer
    /// clearly meant as structure but left unfinished.
    #[must_use]
    pub const fn severity(self) -> Severity {
        match self {
            Self::UnterminatedSection
            | Self::UnclosedQuote
            | Self::TextAfterClosingQuote
            | Self::TextAfterSectionHeader => Severity::Error,
            Self::DuplicateSection | Self::KeyOutsideSection | Self::InvalidEscape => {
                Severity::Warning
            }
        }
    }

    /// The kind behind a wire code, so a host can recover severity from a
    /// bare [`Diagnostic`].
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.code() == code)
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

/// One `[name]` header and the entries it scopes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section<'source> {
    /// The header line, including its record break.
    pub span: Span,
    /// The bytes between the brackets, or `None` for `[]`.
    pub name: Option<&'source str>,
    /// Where [`Section::name`] came from; `None` when the name is empty.
    pub name_span: Option<Span>,
    /// Index range into [`Parse::entries`].
    pub entries: Range<usize>,
    /// Index of the first section sharing this name, when it is a repeat.
    pub duplicate_of: Option<usize>,
    /// Whether the closing `]` was found before the break.
    pub terminated: bool,
}

/// One key/value pair, as raw-byte spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// The entry's bytes from its first key byte to its last value byte.
    pub span: Span,
    /// Key bytes as written, so a quoted key keeps its quotes; `None`
    /// when the line had no key bytes.
    pub key: Option<Span>,
    /// The separator: `=`, `:` or the whitespace run that played its part.
    pub separator: Option<Span>,
    /// Value bytes as written: quotes and a continuation break included,
    /// so a reader always sees the region the author typed.
    pub value: Option<Span>,
    /// The section scoping this entry, or `None` before any header.
    pub section: Option<usize>,
    /// Whether the key was written between quotes (INI only).
    pub key_quoted: bool,
    /// Whether the value was written between quotes (INI only).
    pub value_quoted: bool,
    /// Whether the entry region contains a flagged (malformed) span.
    pub has_error: bool,
}

impl Entry {
    /// Whether this entry carries value bytes at all.
    #[must_use]
    pub const fn has_value(&self) -> bool {
        self.value.is_some()
    }
}

/// Lossless tokens plus INI/properties diagnostics and a flat section/entry
/// table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse<'source> {
    lexed: Lexed<'source>,
    diagnostics: Vec<Diagnostic>,
    sections: Vec<Section<'source>>,
    entries: Vec<Entry>,
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
    pub fn sections(&self) -> &[Section<'source>] {
        &self.sections
    }

    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The entries one section scopes.
    #[must_use]
    pub fn section_entries(&self, section: &Section<'source>) -> &[Entry] {
        self.entries
            .get(section.entries.clone())
            .unwrap_or_default()
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

/// Runs the lossless lexer and the recovering structural pass.
#[must_use]
pub fn parse(source: &str, options: Options) -> Parse<'_> {
    let lexed = lex(source, options);
    let mut diagnostics = Vec::new();
    flagged_diagnostics(&lexed, &mut diagnostics);
    let (mut sections, entries) = read_structure(&lexed, options, &mut diagnostics);
    let ranges = section_ranges(&sections, &entries);
    for (section, range) in sections.iter_mut().zip(ranges) {
        section.entries = range;
    }
    diagnostics.sort_by_key(|diagnostic| diagnostic.span.start);
    Parse {
        lexed,
        diagnostics,
        sections,
        entries,
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
            // An unclosed quoted span flags its opening quote; the abandoned
            // text keeps its key or value kind so the stream stays classifiable.
            SyntaxKind::Quote if token.has_error() => {
                output.push(DiagnosticKind::UnclosedQuote.to_diagnostic(token.span));
            }
            SyntaxKind::Error => {
                let kind = if token.after_section_header() {
                    DiagnosticKind::TextAfterSectionHeader
                } else {
                    DiagnosticKind::TextAfterClosingQuote
                };
                output.push(kind.to_diagnostic(token.span));
            }
            SyntaxKind::EscapeSequence if token.is_invalid_escape() => {
                output.push(DiagnosticKind::InvalidEscape.to_diagnostic(token.span));
            }
            _ => {}
        }
    }
}

/// The section header being read, between its brackets and its record break.
#[derive(Debug, Clone, Copy)]
struct PendingSection {
    start: usize,
    end: usize,
    name: Option<Span>,
    terminated: bool,
}

/// The entry being accumulated between two record breaks.
#[derive(Debug, Default, Clone, Copy)]
struct EntryAccum {
    key: Option<Span>,
    separator: Option<Span>,
    value: Option<Span>,
    key_quoted: bool,
    value_quoted: bool,
    has_error: bool,
}

impl EntryAccum {
    #[must_use]
    fn is_open(self) -> bool {
        self.key.is_some() || self.separator.is_some() || self.value.is_some()
    }

    /// Text joins the value once the separator has been seen, the key before.
    fn add_text(&mut self, token: &LexToken) {
        let in_value = self.separator.is_some();
        let target = if in_value {
            &mut self.value
        } else {
            &mut self.key
        };
        *target = Some(target.map_or(token.span, |span| span.cover(token.span)));
        if in_value {
            self.value_quoted |= token.is_quoted();
        } else {
            self.key_quoted |= token.is_quoted();
        }
        self.has_error |= token.has_error();
    }

    #[must_use]
    fn close(self, section: Option<usize>) -> Entry {
        let mut span = self
            .key
            .or(self.separator)
            .or(self.value)
            .expect("an open entry always has at least one span");
        for part in [self.key, self.separator, self.value].into_iter().flatten() {
            span = span.cover(part);
        }
        Entry {
            span,
            key: self.key,
            separator: self.separator,
            value: self.value,
            section,
            key_quoted: self.key_quoted,
            value_quoted: self.value_quoted,
            has_error: self.has_error,
        }
    }
}

/// Splits the token stream into sections and their entries. A record break
/// flagged [`crate::lexer::TokenFlags::CONTINUED`] does not end the logical
/// line, and a `.properties` continuation never emits a record break at all,
/// so both dialects keep one entry open across physical lines here.
fn read_structure<'source>(
    lexed: &Lexed<'source>,
    options: Options,
    output: &mut Vec<Diagnostic>,
) -> (Vec<Section<'source>>, Vec<Entry>) {
    let source = lexed.source();
    let mut sections: Vec<Section<'source>> = Vec::new();
    let mut entries: Vec<Entry> = Vec::new();
    let mut seen: HashMap<&'source str, usize> = HashMap::new();
    let mut entry = EntryAccum::default();
    let mut pending: Option<PendingSection> = None;
    let mut current: Option<usize> = None;
    let mut reported_global_key = false;

    for token in lexed.tokens() {
        match token.kind {
            SyntaxKind::Bom | SyntaxKind::Padding | SyntaxKind::Comment => {}
            SyntaxKind::SectionMarker => {
                if let Some(raw) = pending.as_mut() {
                    raw.terminated = true;
                    raw.end = token.span.end;
                } else {
                    pending = Some(PendingSection {
                        start: token.span.start,
                        end: token.span.end,
                        name: None,
                        terminated: false,
                    });
                }
            }
            SyntaxKind::SectionName => {
                if let Some(raw) = pending.as_mut() {
                    raw.name = Some(token.span);
                    raw.end = token.span.end;
                }
            }
            SyntaxKind::Error if token.after_section_header() => {
                if let Some(raw) = pending.as_mut() {
                    raw.end = raw.end.max(token.span.end);
                }
            }
            SyntaxKind::Separator => {
                entry.separator = Some(token.span);
                entry.has_error |= token.has_error();
            }
            SyntaxKind::Key
            | SyntaxKind::Value
            | SyntaxKind::Quote
            | SyntaxKind::EscapeSequence
            | SyntaxKind::LineContinuation
            | SyntaxKind::Error => entry.add_text(token),
            SyntaxKind::RecordBreak => {
                if token.continues_line() {
                    continue;
                }
                close_entry(
                    &mut entry,
                    current,
                    options,
                    &mut reported_global_key,
                    &mut entries,
                    output,
                );
                finish_section(
                    &mut pending,
                    source,
                    token.span.end,
                    &mut sections,
                    &mut current,
                    &mut seen,
                    output,
                );
            }
        }
    }
    close_entry(
        &mut entry,
        current,
        options,
        &mut reported_global_key,
        &mut entries,
        output,
    );
    finish_section(
        &mut pending,
        source,
        source.len(),
        &mut sections,
        &mut current,
        &mut seen,
        output,
    );
    (sections, entries)
}

/// Closes the entry a line carried, warning once per document about the first
/// key that lands outside every section.
fn close_entry(
    entry: &mut EntryAccum,
    current: Option<usize>,
    options: Options,
    reported: &mut bool,
    entries: &mut Vec<Entry>,
    output: &mut Vec<Diagnostic>,
) {
    if !entry.is_open() {
        return;
    }
    let closed = std::mem::take(entry).close(current);
    if options.sections && closed.section.is_none() && closed.key.is_some() && !*reported {
        *reported = true;
        let span = closed.key.unwrap_or(closed.span);
        output.push(DiagnosticKind::KeyOutsideSection.to_diagnostic(span));
    }
    entries.push(closed);
}

/// Turns the header line just read into a [`Section`], flagging an
/// unterminated bracket and a repeated name where they occur.
fn finish_section<'source>(
    pending: &mut Option<PendingSection>,
    source: &'source str,
    line_end: usize,
    sections: &mut Vec<Section<'source>>,
    current: &mut Option<usize>,
    seen: &mut HashMap<&'source str, usize>,
    output: &mut Vec<Diagnostic>,
) {
    let Some(raw) = pending.take() else {
        return;
    };
    let name = raw.name.and_then(|span| span.slice(source));
    let duplicate_of = name.and_then(|name| seen.get(name).copied());
    let index = sections.len();
    if let Some(name) = name {
        seen.entry(name).or_insert(index);
    }
    if !raw.terminated {
        output.push(
            DiagnosticKind::UnterminatedSection.to_diagnostic(Span::new(raw.start, line_end)),
        );
    } else if duplicate_of.is_some() {
        output.push(DiagnosticKind::DuplicateSection.to_diagnostic(Span::new(raw.start, raw.end)));
    }
    // Recovery lets an unterminated header scope the lines below it, because
    // that is how every tolerant INI reader resolves those bytes.
    *current = Some(index);
    sections.push(Section {
        span: Span::new(raw.start, line_end),
        name,
        name_span: raw.name,
        entries: 0..0,
        duplicate_of,
        terminated: raw.terminated,
    });
}

/// The entry range each section scopes, in [`Parse::entries`] order.
fn section_ranges(sections: &[Section<'_>], entries: &[Entry]) -> Vec<Range<usize>> {
    let mut ranges = vec![0..0usize; sections.len()];
    for (index, entry) in entries.iter().enumerate() {
        let Some(section) = entry.section else {
            continue;
        };
        let Some(range) = ranges.get_mut(section) else {
            continue;
        };
        if range.end == range.start {
            range.start = index;
        }
        range.end = index + 1;
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Dialect;

    fn codes(source: &str, options: Options) -> Vec<&'static str> {
        validate(source, options)
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect()
    }

    pub(crate) const SAMPLE_INI: &str = concat!(
        "; global note\n",
        "[owner]\n",
        "name = Grace Hopper\n",
        "birth = 1906-12-09\n",
        "quote = \"most of my work has come from being lazy\"\n",
        "  about a good solution ; trailing comment\n",
        "[compiler]\n",
        "version = 3.81\n",
        "enabled = true\n",
        "flags = -O2;-g\n",
        "empty =\n",
    );

    pub(crate) const SAMPLE_PROPERTIES: &str = concat!(
        "# build settings\n",
        "! another comment\n",
        "group.id = 42\n",
        "name\\:full = Grace\\tHopper\n",
        "unicode = \\u00e9\\u0041\n",
        "wrapped = first\\\n",
        "        second\n",
        "url\\:port = http://example.com:8080/x\n",
        "flagged = false\n",
        "colon:value\n",
        "semicolon;key = ; not a comment\n",
    );

    #[test]
    fn valid_representative_documents_have_no_diagnostics() {
        for (source, options) in [
            (SAMPLE_INI, Options::INI),
            (SAMPLE_PROPERTIES, Options::PROPERTIES),
            ("[s]\nk = v\n", Options::INI),
            ("[a]\n[b]\nk = v\n", Options::INI),
            ("k=v\nk2:v2\nk3 v3\n", Options::PROPERTIES),
            ("", Options::INI),
            ("", Options::PROPERTIES),
            ("\n", Options::INI),
            ("[only-section]\n", Options::INI),
            ("[s]\nk\n", Options::INI),
            ("key = value", Options::PROPERTIES),
        ] {
            let parsed = parse(source, options);
            assert!(
                parsed.diagnostics().is_empty(),
                "{source:?} {options:?} {:?}",
                parsed.diagnostics()
            );
            assert!(parsed.is_valid(), "{source:?}");
            assert!(parsed.lexed().is_lossless(), "{source:?}");
        }
    }

    #[test]
    fn sample_documents_have_the_expected_shape() {
        let parsed = parse(SAMPLE_INI, Options::INI);
        let sections = parsed.sections();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].name, Some("owner"));
        assert_eq!(sections[1].name, Some("compiler"));
        assert_eq!(parsed.entries().len(), 7);
        assert_eq!(parsed.section_entries(&sections[0]).len(), 3);
        assert_eq!(parsed.section_entries(&sections[1]).len(), 4);
        assert_eq!(sections[0].entries, 0..3);
        assert_eq!(sections[1].entries, 3..7);
        assert_eq!(parsed.entries()[0].section, Some(0));
        assert!(parsed.entries()[0].key.is_some());
        assert!(!parsed.entries()[6].has_value(), "`empty =` has none");
        assert!(parsed.entries()[2].value_quoted);

        let parsed = parse(SAMPLE_PROPERTIES, Options::PROPERTIES);
        assert_eq!(parsed.sections().len(), 0);
        assert_eq!(parsed.entries().len(), 8);
        assert!(parsed.entries().iter().all(|entry| entry.section.is_none()));
        assert!(parsed.entries().iter().all(|entry| !entry.has_error));
    }

    #[test]
    fn ini_sections_scope_entries_without_a_tree() {
        let parsed = parse("[a]\nx = 1\ny = 2\n[b]\nz = 3\n", Options::INI);
        assert!(parsed.is_valid());
        let sections = parsed.sections();
        assert_eq!(
            sections.iter().map(|s| s.name).collect::<Vec<_>>(),
            vec![Some("a"), Some("b")]
        );
        assert_eq!(
            parsed
                .entries()
                .iter()
                .map(|e| e.section)
                .collect::<Vec<_>>(),
            vec![Some(0), Some(0), Some(1)]
        );
        assert_eq!(sections[0].span, Span::new(0, 4));
        assert_eq!(sections[1].span, Span::new(16, 20));
        assert_eq!(sections[1].name_span, Some(Span::new(17, 18)));
    }

    #[test]
    fn unterminated_section_is_reported_and_still_scopes() {
        let source = "[oops\nk = v\n";
        assert_eq!(codes(source, Options::INI), vec!["unterminated-section"]);
        let parsed = parse(source, Options::INI);
        assert!(!parsed.is_valid());
        assert!(parsed.lexed().is_lossless());
        assert_eq!(parsed.lexed().joined(), source);
        assert!(!parsed.sections()[0].terminated);
        assert_eq!(parsed.sections()[0].span, Span::new(0, 6));
        assert_eq!(parsed.sections()[0].name, Some("oops"));
        assert_eq!(parsed.entries()[0].section, Some(0));
        // The same bytes are one ordinary key in .properties.
        assert_eq!(
            codes(source, Options::PROPERTIES),
            Vec::<&str>::new(),
            "no sections means no unterminated ones"
        );
        assert_eq!(parse(source, Options::PROPERTIES).entries().len(), 2);
    }

    #[test]
    fn duplicate_section_is_a_warning_that_keeps_the_document_valid() {
        let source = "[a]\nx = 1\n[a]\ny = 2\n";
        assert_eq!(codes(source, Options::INI), vec!["duplicate-section"]);
        let parsed = parse(source, Options::INI);
        assert!(parsed.is_valid(), "a warning is not an error");
        assert_eq!(parsed.sections()[1].duplicate_of, Some(0));
        assert_eq!(parsed.sections()[0].duplicate_of, None);
        assert_eq!(parsed.section_entries(&parsed.sections()[1]).len(), 1);
        assert_eq!(
            parsed.diagnostics()[0].span,
            Span::new(10, 13),
            "reported on the repeat header"
        );
    }

    #[test]
    fn a_key_before_any_section_warns_once_per_document() {
        let source = "a = 1\nb = 2\n[s]\nc = 3\n";
        assert_eq!(codes(source, Options::INI), vec!["key-outside-section"]);
        let parsed = parse(source, Options::INI);
        assert!(parsed.is_valid());
        assert_eq!(parsed.entries()[0].section, None);
        assert_eq!(parsed.entries()[2].section, Some(0));
        assert_eq!(
            parsed.diagnostics()[0].span,
            Span::new(0, 1),
            "one finding, on the first global key"
        );
        // Sections are INI-only structure, so properties cannot warn here.
        assert_eq!(codes(source, Options::PROPERTIES), Vec::<&str>::new());
    }

    #[test]
    fn unclosed_quote_is_reported_once_and_the_line_recovers() {
        let source = "[s]\nk = \"abc\nnext = 1\n";
        assert_eq!(codes(source, Options::INI), vec!["unclosed-quote"]);
        let parsed = parse(source, Options::INI);
        assert!(!parsed.is_valid());
        assert!(parsed.lexed().is_lossless());
        assert_eq!(parsed.entries().len(), 2);
        assert!(parsed.entries()[0].has_error);
        assert!(parsed.entries()[0].value_quoted);
        assert!(!parsed.entries()[1].has_error);
    }

    #[test]
    fn text_after_a_closing_quote_keeps_its_span() {
        let source = "[s]\nk = \"v\" junk more\n";
        assert_eq!(
            codes(source, Options::INI),
            vec!["text-after-closing-quote"]
        );
        let parsed = parse(source, Options::INI);
        assert!(!parsed.is_valid());
        assert_eq!(parsed.diagnostics()[0].span, Span::new(12, 21));
        assert!(parsed.entries()[0].has_error);
    }

    #[test]
    fn text_after_a_section_header_keeps_its_span() {
        let source = "[s] trailing junk\nk = v\n";
        assert_eq!(
            codes(source, Options::INI),
            vec!["text-after-section-header"]
        );
        let parsed = parse(source, Options::INI);
        assert!(!parsed.is_valid());
        assert_eq!(parsed.diagnostics()[0].span, Span::new(4, 17));
        assert_eq!(parsed.entries().len(), 1);
        assert_eq!(parsed.entries()[0].section, Some(0));
    }

    #[test]
    fn unknown_escapes_warn_without_breaking_the_document() {
        let source = "a = \\q\nb = \\u12z4\nc = \\n\n";
        assert_eq!(
            codes(source, Options::PROPERTIES),
            vec!["invalid-escape", "invalid-escape"]
        );
        let parsed = parse(source, Options::PROPERTIES);
        assert!(parsed.is_valid());
        assert_eq!(parsed.entries().len(), 3);
        // INI has no escapes, so the same bytes are clean value text.
        assert_eq!(
            codes("[s]\na = \\q\n", Options::INI),
            Vec::<&str>::new(),
            "no escapes means no invalid-escape finding"
        );
    }

    #[test]
    fn recovery_keeps_every_diagnostic_on_real_bytes() {
        let broken = concat!(
            "\u{FEFF}global = 1\n",
            "[unclosed\n",
            "k = \"oops\" tail\n",
            "[s]\n",
            "esc = value\n",
        );
        let parsed = parse(broken, Options::INI);
        assert!(parsed.lexed().is_lossless());
        assert_eq!(parsed.lexed().joined(), broken);
        assert!(!parsed.is_valid());
        let found = codes(broken, Options::INI);
        assert!(found.contains(&"key-outside-section"), "{found:?}");
        assert!(found.contains(&"unterminated-section"), "{found:?}");
        assert!(found.contains(&"text-after-closing-quote"), "{found:?}");
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
        let source = "[a\n[s]\nk = \"v\" junk\n[s]\n";
        let diagnostics = validate(source, Options::INI);
        assert!(
            diagnostics
                .windows(2)
                .all(|pair| pair[0].span.start <= pair[1].span.start),
            "{diagnostics:?}"
        );
        assert_eq!(
            diagnostics.iter().map(|d| d.code).collect::<Vec<_>>(),
            vec![
                "unterminated-section",
                "text-after-closing-quote",
                "duplicate-section"
            ]
        );
    }

    #[test]
    fn continuation_entries_are_one_entry_not_two() {
        let parsed = parse("[s]\nk = one\n  two\n  three\n", Options::INI);
        assert!(parsed.is_valid());
        assert_eq!(parsed.entries().len(), 1);
        let entry = &parsed.entries()[0];
        assert_eq!(entry.value, Some(Span::new(8, 25)));
        assert!(entry.has_value());
        assert_eq!(entry.span, Span::new(4, 25));

        let parsed = parse("k = one\\\n  two\nnext = 2\n", Options::PROPERTIES);
        assert!(parsed.is_valid());
        assert_eq!(parsed.entries().len(), 2);
        assert_eq!(parsed.entries()[0].value, Some(Span::new(4, 14)));
        assert_eq!(parsed.entries()[0].key, Some(Span::new(0, 1)));
        assert_eq!(parsed.entries()[1].key, Some(Span::new(15, 19)));
    }

    #[test]
    fn keys_and_separators_are_recorded_as_written() {
        let parsed = parse("name\\:full = a\\tb\nlonely\n", Options::PROPERTIES);
        assert!(parsed.is_valid());
        let entries = parsed.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, Some(Span::new(0, 10)));
        assert_eq!(entries[0].separator, Some(Span::new(11, 12)));
        assert_eq!(entries[0].value, Some(Span::new(13, 17)));
        assert_eq!(entries[1].key, Some(Span::new(18, 24)));
        assert_eq!(entries[1].separator, None);
        assert!(!entries[1].has_value());

        let parsed = parse("[s]\n\"quoted key\" = 1\n= orphan\n", Options::INI);
        assert!(parsed.is_valid());
        assert_eq!(parsed.entries()[0].key, Some(Span::new(4, 16)));
        assert_eq!(parsed.entries()[0].separator, Some(Span::new(17, 18)));
        assert!(parsed.entries()[0].key_quoted);
        assert_eq!(parsed.entries()[1].key, None);
        assert_eq!(parsed.entries()[1].separator, Some(Span::new(21, 22)));
    }

    #[test]
    fn every_truncation_still_produces_consistent_structure() {
        let ini = "[s]\nkey = \"a;b\" ; c\ncont = one\n  two\n[unterminated\nplain\n= orphan\n";
        let properties = "a\\:b = x\\u0041y\\\n  cont\nplain\ncolon: value\n";
        for (sample, options) in [(ini, Options::INI), (properties, Options::PROPERTIES)] {
            for cut in 0..=sample.len() {
                let source = &sample[..cut];
                let parsed = parse(source, options);
                assert!(parsed.lexed().is_lossless(), "{source:?}");
                assert_eq!(parsed.lexed().joined(), source, "{source:?}");
                let total: usize = parsed
                    .sections()
                    .iter()
                    .map(|section| parsed.section_entries(section).len())
                    .sum();
                let scoped = parsed
                    .entries()
                    .iter()
                    .filter(|entry| entry.section.is_some())
                    .count();
                assert_eq!(total, scoped, "{source:?}");
                for section in parsed.sections() {
                    assert!(
                        section.span.is_valid_for(source) && !section.span.is_empty(),
                        "{source:?} has a bad section span"
                    );
                    for span in [section.name_span, Some(section.span)] {
                        assert!(
                            span.is_none_or(|span| span.is_valid_for(source)),
                            "{source:?} escapes the source"
                        );
                    }
                }
                for entry in parsed.entries() {
                    assert!(!entry.span.is_empty(), "{source:?}");
                    assert!(entry.span.is_valid_for(source), "{source:?}");
                    for span in [entry.key, entry.separator, entry.value] {
                        assert!(
                            span.is_none_or(|span| !span.is_empty() && span.is_valid_for(source)),
                            "{source:?} has a zero-width entry span"
                        );
                    }
                }
                for diagnostic in parsed.diagnostics() {
                    assert!(
                        diagnostic.span.is_valid_for(source) && !diagnostic.span.is_empty(),
                        "{source:?} {diagnostic:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn empty_and_comment_only_documents_have_no_entries() {
        for (source, options) in [
            ("", Options::INI),
            ("\n", Options::INI),
            ("\n\n", Options::INI),
            ("; only a comment\n", Options::INI),
            ("# c\n# d\n", Options::INI),
            ("", Options::PROPERTIES),
            ("\n", Options::PROPERTIES),
            ("\n\n", Options::PROPERTIES),
            ("# c\n! c\n", Options::PROPERTIES),
        ] {
            let parsed = parse(source, options);
            assert!(parsed.is_valid(), "{source:?}");
            assert!(
                parsed.sections().is_empty() && parsed.entries().is_empty(),
                "{source:?} {options:?} sections={:?} entries={:?}",
                parsed.sections(),
                parsed.entries()
            );
            assert!(parsed.lexed().is_lossless(), "{source:?}");
        }
    }

    #[test]
    fn severity_and_codes_round_trip() {
        assert_eq!(DiagnosticKind::ALL.len(), 7);
        for kind in DiagnosticKind::ALL {
            assert_eq!(DiagnosticKind::from_code(kind.code()), Some(kind));
            assert_eq!(
                themoretheless_tokenizer_core::DiagnosticKind::code(kind),
                kind.code(),
                "the trait and the inherent code must agree"
            );
            assert_eq!(
                themoretheless_tokenizer_core::DiagnosticKind::severity(kind),
                kind.severity()
            );
            assert_eq!(format!("{kind}"), kind.message(), "Display is the message");
            assert!(!kind.message().is_empty());
            assert!(kind.code().contains('-'), "{}", kind.code());
            let diagnostic = kind.to_diagnostic(Span::new(0, 1));
            assert_eq!(diagnostic.code, kind.code());
            assert_eq!(diagnostic.message, kind.message());
        }
        assert_eq!(DiagnosticKind::from_code("nope"), None);
        assert_eq!(
            DiagnosticKind::DuplicateSection.severity(),
            Severity::Warning
        );
        assert_eq!(
            DiagnosticKind::UnterminatedSection.severity(),
            Severity::Error
        );
        assert_eq!(DiagnosticKind::InvalidEscape.severity(), Severity::Warning);
        assert_eq!(
            DiagnosticKind::TextAfterSectionHeader.severity(),
            Severity::Error
        );
        assert_eq!(
            DiagnosticKind::from_code("unclosed-quote").map(DiagnosticKind::severity),
            Some(Severity::Error)
        );
    }

    #[test]
    fn dialect_helpers_pick_the_same_options_as_the_lexer() {
        assert_eq!(Dialect::Ini.options(), Options::INI);
        assert_eq!(Dialect::Properties.options(), Options::PROPERTIES);
        assert_eq!(parse("[a]\n", Options::INI).sections()[0].name, Some("a"));
        assert!(parse("[a]\n", Options::PROPERTIES).sections().is_empty());
    }
}
