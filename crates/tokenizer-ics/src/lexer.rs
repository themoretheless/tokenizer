//! A lossless iCalendar (`.ics`) lexer.
//!
//! Concatenating the token text reconstructs the source byte-for-byte — for
//! well-formed calendars and for broken ones alike. Nothing is dropped, merged
//! into a neighbour, or synthesized: a line that never reaches its `:` keeps its
//! own flagged span, an unterminated quoted parameter keeps its own, and a fold
//! continuation whose line has nothing to say keeps its own.
//!
//! The grammar is RFC 5545's content-line grammar, with one deliberate division
//! of labour: this lexer is *physical*, [`crate::parser`] is *logical*.
//!
//! ```text
//!  contentline    = name *(";" param) ":" value CRLF
//!  name           = iana-token / x-name            ; ALPHA / DIGIT / "-" ("_" too)
//!  param          = param-name "=" param-value *("," param-value)
//!  param-value    = paramtext | quoted-string
//!  quoted-string  = DQUOTE *QSAFE-CHAR DQUOTE
//! ```
//!
//! Because the lexer works physically, a fold — `CRLF` followed by one space or
//! horizontal tab — is *not* undone here. The break plus the single whitespace
//! byte that introduces a continuation line is emitted as its own
//! [`SyntaxKind::FoldMarker`] token, and every other span stays inside one
//! physical line. That is what keeps the contract honest: a value split over
//! three physical lines still yields spans that are regions of the raw bytes,
//! and the parser re-joins the lines structurally without moving a single span.
//!
//! Line endings are RFC 5545's `CRLF`. A lone `LF` or `CR` is invalid per the
//! specification but must still terminate a line so that nothing is swallowed:
//! it becomes a flagged [`SyntaxKind::LineBreak`]. There is no comment kind
//! anywhere in this lexer, because iCalendar has no comments.

use themoretheless_tokenizer_core::{LosslessViolation, Span, verify_lossless_spans};

/// Exact lexical categories emitted by the iCalendar lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxKind {
    /// The UTF-8 encoding of U+FEFF at document start.
    Bom,
    /// `BEGIN` or `END`: the structural marker word of a component line.
    StructureMarker,
    /// A component name (`VCALENDAR`, `VEVENT`, `VALARM`) or an ordinary
    /// property name (`SUMMARY`, `X-VENDOR-PROP`). The bytes are the same; the
    /// parser decides which is which from the marker in front of it.
    PropertyName,
    /// A parameter name (`TZID`, `VALUE`, `CN`, `DELEGATED-FROM`).
    ParameterName,
    /// The `:` that closes a name and opens its value.
    ValueColon,
    /// A `;` between two parameters.
    ParameterSeparator,
    /// An `=` between a parameter name and its value.
    ParameterEquals,
    /// A `,` between two values of one parameter.
    ValueComma,
    /// A double-quoted parameter value, both quotes included. RFC 5545 offers no
    /// escape for `DQUOTE` inside `quoted-string`, so the region ends at the
    /// first quote after the opening one (RFC 6868's parameter escaping is out
    /// of scope here).
    QuotedParam,
    /// An unquoted parameter value, up to the next `,` or the end of its
    /// parameter.
    BareParam,
    /// A property value: the bytes between the `:` and the end of the physical
    /// line that are not part of an escape sequence. iCalendar types values by
    /// property and escapes only inside text, so the value's *reading* is the
    /// parser's job; only the two-byte escapes are carved out here.
    Value,
    /// A `\,`, `\;`, `\\` or `\n`/`\N` sequence inside a value. A backslash
    /// followed by anything else is not an RFC 5545 escape and is flagged.
    Escape,
    /// A `CRLF` — or a bare `LF`/`CR`, which carries the error flag — ending a
    /// physical line.
    LineBreak,
    /// A break plus the single space or tab that folds the next line into the
    /// previous one.
    FoldMarker,
    /// A byte run the grammar has no place for: a line with no value delimiter,
    /// a line with no name, or a parameter that is not `name=value`.
    Error,
}

impl SyntaxKind {
    /// Trivia separates content lines without carrying calendar content.
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(self, Self::Bom | Self::LineBreak | Self::FoldMarker)
    }

    /// Whether a byte may appear in a name or parameter name: `IANA-Token`
    /// plus the `_` that some vendor prefixes use.
    #[must_use]
    pub const fn is_name_byte(byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
    }
}

/// Compact per-token state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TokenFlags(u8);

impl TokenFlags {
    pub const EMPTY: Self = Self(0);
    /// The span is part of, or abandoned by, a malformed construct.
    pub const HAS_ERROR: Self = Self(1);

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// A lexical token. Spans are non-empty UTF-8 byte ranges and never straddle a
/// fold, so every token is a region of exactly one physical line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LexToken {
    pub kind: SyntaxKind,
    pub span: Span,
    pub flags: TokenFlags,
}

impl LexToken {
    #[must_use]
    pub fn text(self, source: &str) -> Option<&str> {
        source.get(self.span.range())
    }

    /// Whether this token belongs to a malformed construct.
    #[must_use]
    pub const fn has_error(self) -> bool {
        self.flags.contains(TokenFlags::HAS_ERROR)
    }
}

/// Lossless lexer output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lexed<'source> {
    source: &'source str,
    tokens: Vec<LexToken>,
}

impl<'source> Lexed<'source> {
    #[must_use]
    pub const fn source(&self) -> &'source str {
        self.source
    }

    #[must_use]
    pub fn tokens(&self) -> &[LexToken] {
        &self.tokens
    }

    /// Tokens that are not a BOM, a line break or a fold marker.
    pub fn significant_tokens(&self) -> impl Iterator<Item = LexToken> + '_ {
        self.tokens
            .iter()
            .copied()
            .filter(|token| !token.kind.is_trivia())
    }

    #[must_use]
    pub fn text(&self, token: LexToken) -> Option<&'source str> {
        token.text(self.source)
    }

    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.tokens.iter().any(|token| token.has_error())
    }

    /// Token text concatenated in span order.
    #[must_use]
    pub fn joined(&self) -> String {
        let mut joined = String::with_capacity(self.source.len());
        for token in &self.tokens {
            if let Some(text) = token.text(self.source) {
                joined.push_str(text);
            }
        }
        joined
    }

    /// Whether the token stream covers the source with no gaps or overlaps.
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        self.verify_lossless().is_ok()
    }

    /// Named lossless violation. Streams from [`lex`] always pass; the check is
    /// public so a host can re-verify a transformed token list.
    pub fn verify_lossless(&self) -> Result<(), LosslessViolation> {
        verify_lossless_spans(self.source, self.tokens.iter().map(|token| token.span))
    }
}

/// UTF-8 encoding of U+FEFF.
const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// Lexes an iCalendar document into physical content lines.
#[must_use]
pub fn lex(source: &str) -> Lexed<'_> {
    let mut lexer = Lexer {
        bytes: source.as_bytes(),
        pos: 0,
        tokens: Vec::new(),
        continuation: false,
    };
    lexer.run();
    Lexed {
        source,
        tokens: lexer.tokens,
    }
}

struct Lexer<'source> {
    bytes: &'source [u8],
    pos: usize,
    tokens: Vec<LexToken>,
    /// Whether the physical line about to be read continues the previous one,
    /// i.e. it was introduced by a fold marker. A continuation carries value
    /// bytes only: unfolding has already decided where its name and delimiter
    /// are, and re-scanning it as a fresh content line would invent structure
    /// the document does not have.
    continuation: bool,
}

/// How one physical line's structure walk ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Walk {
    /// The line was decomposed into structured tokens, appended to the buffer.
    Parsed,
    /// The bytes before any structure the grammar recognises: the caller keeps
    /// the whole line as one flagged run.
    Malformed,
}

impl Lexer<'_> {
    fn run(&mut self) {
        if self.bytes.starts_with(&BOM) {
            self.push(SyntaxKind::Bom, 0, BOM.len(), TokenFlags::EMPTY);
            self.pos = BOM.len();
        }
        while self.pos < self.bytes.len() {
            self.lex_physical_line();
        }
    }

    /// One physical line and the break or fold marker that ends it. Every call
    /// leaves `pos` strictly greater than it was: the break or fold branch
    /// consumes at least its break byte, and an empty line still has one.
    fn lex_physical_line(&mut self) {
        let start = self.pos;
        let content_end = self.line_end(start);
        if self.continuation {
            let mut planned = Vec::new();
            lex_value(self.bytes, start, content_end, &mut planned);
            self.tokens.append(&mut planned);
        } else {
            self.lex_content(start, content_end);
        }
        self.pos = content_end;
        self.lex_break_or_fold();
    }

    fn line_end(&self, from: usize) -> usize {
        let mut end = from;
        while end < self.bytes.len() && !is_break(self.bytes[end]) {
            end += 1;
        }
        end
    }

    /// `name *(";" param) ":" value`. An empty line contributes no token: it has
    /// no bytes, and a zero-width span is forbidden.
    fn lex_content(&mut self, start: usize, stop: usize) {
        if start >= stop {
            return;
        }
        let mut planned = Vec::new();
        if self.walk_line(start, stop, &mut planned) == Walk::Parsed {
            self.tokens.append(&mut planned);
            return;
        }
        // One flagged run for the whole line: every byte survives, the parser
        // names the fault, and no token claims a structure the line never
        // reached.
        self.push(SyntaxKind::Error, start, stop, TokenFlags::HAS_ERROR);
    }

    /// The single structure walk for a line: it both locates the value
    /// delimiter and names every parameter region, so the two can never drift
    /// apart and a `;` inside a quoted parameter value cannot split one
    /// parameter into two. Tokens are staged in `out`, because the walk only
    /// learns the line is malformed while it is still holding nothing back.
    fn walk_line(&self, start: usize, stop: usize, out: &mut Vec<LexToken>) -> Walk {
        let name_end = name_run(self.bytes, start, stop);
        if name_end == start {
            return Walk::Malformed;
        }
        emit_name(self.bytes, start, name_end, out);
        let mut cursor = name_end;
        loop {
            if cursor >= stop {
                // The line ran out of bytes without a value delimiter. Its name
                // and parameters were well formed, so they are reported as what
                // they are and the parser complains about the missing `:`.
                return Walk::Parsed;
            }
            match self.bytes[cursor] {
                b':' => {
                    emit_token(SyntaxKind::ValueColon, cursor, cursor + 1, out);
                    lex_value(self.bytes, cursor + 1, stop, out);
                    return Walk::Parsed;
                }
                b';' => {
                    let separator = cursor;
                    emit_token(
                        SyntaxKind::ParameterSeparator,
                        separator,
                        separator + 1,
                        out,
                    );
                    cursor = self.walk_parameter(separator + 1, stop, out);
                }
                _ => return Walk::Malformed,
            }
        }
    }

    /// One parameter after a `;`, returning where the parameter stopped. A region
    /// that is not `name=value` becomes a single flagged run, so no byte of a
    /// half-written parameter is lost.
    fn walk_parameter(&self, start: usize, stop: usize, out: &mut Vec<LexToken>) -> usize {
        let name_start = start;
        let mut cursor = name_run(self.bytes, start, stop);
        let name_end = cursor;
        if name_end == name_start || cursor >= stop || self.bytes[cursor] != b'=' {
            // Not a parameter this grammar knows.
            let mut end = name_end;
            while end < stop && !matches!(self.bytes[end], b';' | b':') {
                end += 1;
            }
            emit_token_with_flags(
                SyntaxKind::Error,
                name_start,
                end,
                TokenFlags::HAS_ERROR,
                out,
            );
            if end == name_start {
                // An empty region: the `;` already carried its own byte.
                return name_start;
            }
            return end;
        }
        emit_token(SyntaxKind::ParameterName, name_start, name_end, out);
        emit_token(SyntaxKind::ParameterEquals, cursor, cursor + 1, out);
        let assignment = out.len() - 1;
        cursor += 1;
        loop {
            if cursor < stop && self.bytes[cursor] == b'"' {
                let opened = cursor;
                cursor += 1;
                while cursor < stop && self.bytes[cursor] != b'"' {
                    cursor += 1;
                }
                if cursor >= stop {
                    // A quoted value with no closing quote owns the rest of the
                    // physical line and stops the structure walk there.
                    emit_token_with_flags(
                        SyntaxKind::QuotedParam,
                        opened,
                        stop,
                        TokenFlags::HAS_ERROR,
                        out,
                    );
                    return stop;
                }
                cursor += 1;
                emit_token(SyntaxKind::QuotedParam, opened, cursor, out);
            } else {
                let opened = cursor;
                while cursor < stop && !matches!(self.bytes[cursor], b',' | b';' | b':') {
                    cursor += 1;
                }
                if cursor == opened {
                    // `param-value` is `paramtext | quoted-string`, and neither
                    // can be empty, so the assignment itself carries the flag:
                    // there is no value byte to own it and a zero-width span
                    // would break losslessness.
                    out[assignment].flags = TokenFlags::HAS_ERROR;
                } else {
                    emit_token(SyntaxKind::BareParam, opened, cursor, out);
                }
            }
            if cursor < stop && self.bytes[cursor] == b',' {
                emit_token(SyntaxKind::ValueComma, cursor, cursor + 1, out);
                cursor += 1;
                continue;
            }
            return cursor;
        }
    }

    /// The `CRLF` — or the fold marker — that ends the current physical line.
    fn lex_break_or_fold(&mut self) {
        let start = self.pos;
        if start >= self.bytes.len() {
            return;
        }
        let break_end = break_end(self.bytes, start);
        if break_end < self.bytes.len() && is_fold_space(self.bytes[break_end]) {
            self.push(
                SyntaxKind::FoldMarker,
                start,
                break_end + 1,
                TokenFlags::EMPTY,
            );
            self.pos = break_end + 1;
            self.continuation = true;
            return;
        }
        self.continuation = false;
        let crlf = self.bytes[start] == b'\r' && break_end - start == 2;
        self.push(
            SyntaxKind::LineBreak,
            start,
            break_end,
            if crlf {
                TokenFlags::EMPTY
            } else {
                TokenFlags::HAS_ERROR
            },
        );
        self.pos = break_end;
    }

    /// Emits a token. Zero-width spans are skipped so the stream never carries
    /// an empty token: an empty value or an empty parameter contributes nothing.
    fn push(&mut self, kind: SyntaxKind, start: usize, end: usize, flags: TokenFlags) {
        if end <= start {
            return;
        }
        self.tokens.push(LexToken {
            kind,
            span: Span::new(start, end),
            flags,
        });
    }
}

/// The value region: literal runs with the RFC 5545 text escapes carved out as
/// separate tokens, paired left to right so `\\,` reads as an escaped backslash
/// followed by a comma rather than an escaped comma. A backslash that opens no
/// valid escape is kept and flagged.
fn lex_value(bytes: &[u8], start: usize, stop: usize, out: &mut Vec<LexToken>) {
    let mut cursor = start;
    let mut run = start;
    while cursor < stop {
        if bytes[cursor] != b'\\' {
            cursor += 1;
            continue;
        }
        emit_token(SyntaxKind::Value, run, cursor, out);
        if cursor + 1 < stop {
            let valid = matches!(bytes[cursor + 1], b',' | b';' | b'\\' | b'n' | b'N');
            out.push(LexToken {
                kind: SyntaxKind::Escape,
                span: Span::new(cursor, cursor + 2),
                flags: if valid {
                    TokenFlags::EMPTY
                } else {
                    TokenFlags::HAS_ERROR
                },
            });
            cursor += 2;
        } else {
            out.push(LexToken {
                kind: SyntaxKind::Escape,
                span: Span::new(cursor, stop),
                flags: TokenFlags::HAS_ERROR,
            });
            cursor = stop;
        }
        run = cursor;
    }
    emit_token(SyntaxKind::Value, run, stop, out);
}

/// `BEGIN` and `END` are structural markers only when they are the whole name;
/// anything else that opens a line is a property name. The comparison ignores
/// case, because the specification makes names case-insensitive and only asks
/// writers to upper-case them — a request the parser turns into a warning.
fn emit_name(bytes: &[u8], start: usize, end: usize, out: &mut Vec<LexToken>) {
    let head = &bytes[start..end];
    let marker = head.len() == 5 && head.eq_ignore_ascii_case(b"BEGIN");
    let marker = marker || (head.len() == 3 && head.eq_ignore_ascii_case(b"END"));
    let kind = if marker {
        SyntaxKind::StructureMarker
    } else {
        SyntaxKind::PropertyName
    };
    emit_token(kind, start, end, out);
}

/// The end of the name run starting at `from`, bounded by `stop`.
fn name_run(bytes: &[u8], from: usize, stop: usize) -> usize {
    let mut cursor = from;
    while cursor < stop && SyntaxKind::is_name_byte(bytes[cursor]) {
        cursor += 1;
    }
    cursor
}

fn emit_token(kind: SyntaxKind, start: usize, end: usize, out: &mut Vec<LexToken>) {
    emit_token_with_flags(kind, start, end, TokenFlags::EMPTY, out);
}

fn emit_token_with_flags(
    kind: SyntaxKind,
    start: usize,
    end: usize,
    flags: TokenFlags,
    out: &mut Vec<LexToken>,
) {
    if end <= start {
        return;
    }
    out.push(LexToken {
        kind,
        span: Span::new(start, end),
        flags,
    });
}

/// A `CRLF`, or a lone `LF`/`CR`.
fn break_end(bytes: &[u8], start: usize) -> usize {
    let mut end = start + 1;
    if bytes[start] == b'\r' && bytes.get(end) == Some(&b'\n') {
        end += 1;
    }
    end
}

const fn is_break(byte: u8) -> bool {
    byte == b'\n' || byte == b'\r'
}

const fn is_fold_space(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Calendars a real writer could produce: every one must lex clean and
    /// byte-exactly.
    const CLEAN_CORPUS: &[&str] = &[
        "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//x//y//EN\r\nEND:VCALENDAR\r\n",
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:1@host\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        "DTSTAMP:20260924T090000Z\r\n",
        "DTSTART;TZID=Europe/Moscow:20260924T090000\r\n",
        "DTEND;VALUE=DATE:20260925\r\n",
        "DURATION:PT1H30M\r\n",
        "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE,FR\r\n",
        "SUMMARY;LANGUAGE=en:Hello\r\n",
        "ATTENDEE;CN=\"Doe, John\";ROLE=REQ-PARTICIPANT:mailto:a@b.c\r\n",
        "DESCRIPTION:line one\\nline two\\, comma\r\n",
        "GEO:37.386013;-122.082932\r\n",
        "FREEBUSY:20260924T090000Z/20260924T100000Z\r\n",
        "CATEGORIES:x\r\n",
        "X-VENDOR-PROP:value\r\n",
        "X-WR-CALNAME:My Cal\r\n",
        "DESCRIPTION:long line that\r\n continues on the next one\r\n",
        "DESCRIPTION:folded twice\r\n once\r\n and again\r\n",
        "SUMMARY:名前 😀\r\n",
        "SUMMARY:trailing space \r\n",
        "A;B=1;C=2:D\r\n",
        "DTSTART;TZID=\"Europe/Moscow\":20260924T090000\r\n",
        "\u{FEFF}BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
        "END:VCALENDAR",
        "SUMMARY:\r\n",
        "BEGIN:VCALENDAR\r\n\ttab folded\r\nEND:VCALENDAR\r\n",
    ];

    /// Broken or unusual input: still byte-for-byte recoverable.
    const BROKEN_CORPUS: &[&str] = &[
        "BEGIN:VCALENDAR\nEND:VCALENDAR\n",
        "no colon here\r\n",
        ":",
        ";",
        "\"",
        "=",
        ",",
        "\r\n",
        "\n",
        "\r",
        " ",
        "\t",
        "  \r\n",
        " \r\n",
        "\r\n \r\n",
        "\r\n x",
        "A:1\r\n \r\n",
        "A:1\n \n",
        "A:1\r\n ",
        "A:1\r\n  x",
        "BEGIN\r\n",
        "BEGIN:\r\n",
        "BEGIN:VEVENT\r\n",
        "SUMMARY\r\n",
        "vevent:x\r\n",
        "SUM X:1\r\n",
        "BEGINX:1\r\n",
        "END:VTODO\r\n",
        "DTSTART;TZID:20260924T090000\r\n",
        "DTSTART;TZID=:20260924T090000\r\n",
        "DTSTART;=x:y\r\n",
        "DTSTART;;:x\r\n",
        "A;;B=1:x\r\n",
        "A;B=1,:x\r\n",
        "A;B=\"unterminated:x\r\n",
        "A;B=\"oops\r\n",
        "A;B=\"x\\\"y\":v\r\n",
        "A;B=\"x\",\"y\":v\r\n",
        "DTSTART;VALUE=Bogus:x\r\n",
        "DESCRIPTION:a\\qb\r\n",
        "CATEGORIES:a,,b\r\n",
        ";;;\r\n",
        ":value\r\n",
        "\"quoted line\"\r\n",
        "SUMMARY:a\r b\r\n",
        "X:1\r\n \r\n",
        "\u{FEFF}",
        "\u{FEFF}\r\n x",
    ];

    fn assert_lossless(source: &str) -> Lexed<'_> {
        let lexed = lex(source);
        assert!(
            lexed.is_lossless(),
            "{source:?}: {:?}",
            lexed.verify_lossless()
        );
        assert_eq!(lexed.joined(), source, "{source:?}");
        let mut previous = 0;
        for token in lexed.tokens() {
            assert!(!token.span.is_empty(), "{source:?} has a zero-width span");
            assert!(token.span.start >= previous, "{source:?} overlaps");
            assert!(source.is_char_boundary(token.span.start), "{source:?}");
            assert!(source.is_char_boundary(token.span.end), "{source:?}");
            previous = token.span.end;
        }
        lexed
    }

    fn kinds(source: &str) -> Vec<SyntaxKind> {
        lex(source)
            .tokens()
            .iter()
            .map(|token| token.kind)
            .collect()
    }

    fn kinds_and_text(source: &str) -> Vec<(SyntaxKind, &str)> {
        assert_lossless(source);
        lex(source)
            .tokens()
            .iter()
            .map(|token| (token.kind, token.text(source).unwrap()))
            .collect()
    }

    #[test]
    fn corpus_is_lossless_and_terminates() {
        for source in CLEAN_CORPUS.iter().chain(BROKEN_CORPUS) {
            let lexed = assert_lossless(source);
            let mut previous = 0usize;
            for token in lexed.tokens() {
                assert!(token.span.end > token.span.start, "no progress");
                assert!(token.span.start >= previous);
                previous = token.span.end;
            }
            assert_eq!(previous, source.len(), "{source:?} uncovered tail");
        }
    }

    #[test]
    fn a_simple_property_partitions_exactly() {
        assert_eq!(
            kinds_and_text("SUMMARY:Hello\r\n"),
            vec![
                (SyntaxKind::PropertyName, "SUMMARY"),
                (SyntaxKind::ValueColon, ":"),
                (SyntaxKind::Value, "Hello"),
                (SyntaxKind::LineBreak, "\r\n"),
            ]
        );
    }

    #[test]
    fn begin_and_end_are_markers_and_the_component_name_is_their_value() {
        // The component name is, byte for byte, the value of the `BEGIN`
        // property. Retagging it as a component name needs the marker in front
        // of it, so it is the parser that makes that call, not the lexer.
        assert_eq!(
            kinds_and_text("BEGIN:VEVENT\r\n"),
            vec![
                (SyntaxKind::StructureMarker, "BEGIN"),
                (SyntaxKind::ValueColon, ":"),
                (SyntaxKind::Value, "VEVENT"),
                (SyntaxKind::LineBreak, "\r\n"),
            ]
        );
        assert_eq!(
            kinds_and_text("END:VCALENDAR\r\n"),
            vec![
                (SyntaxKind::StructureMarker, "END"),
                (SyntaxKind::ValueColon, ":"),
                (SyntaxKind::Value, "VCALENDAR"),
                (SyntaxKind::LineBreak, "\r\n"),
            ]
        );
        // A name that merely starts with the marker word is not a marker.
        assert_eq!(
            kinds("BEGINX:1\r\n"),
            vec![
                SyntaxKind::PropertyName,
                SyntaxKind::ValueColon,
                SyntaxKind::Value,
                SyntaxKind::LineBreak,
            ]
        );
    }

    #[test]
    fn parameters_are_typed_name_equals_value() {
        assert_eq!(
            kinds_and_text("DTSTART;TZID=Europe/Moscow:20260924T090000\r\n"),
            vec![
                (SyntaxKind::PropertyName, "DTSTART"),
                (SyntaxKind::ParameterSeparator, ";"),
                (SyntaxKind::ParameterName, "TZID"),
                (SyntaxKind::ParameterEquals, "="),
                (SyntaxKind::BareParam, "Europe/Moscow"),
                (SyntaxKind::ValueColon, ":"),
                (SyntaxKind::Value, "20260924T090000"),
                (SyntaxKind::LineBreak, "\r\n"),
            ]
        );
    }

    #[test]
    fn quoted_parameter_values_keep_their_quotes() {
        assert_eq!(
            kinds_and_text("ATTENDEE;CN=\"Doe, John\";ROLE=REQ:mailto:a@b.c\r\n"),
            vec![
                (SyntaxKind::PropertyName, "ATTENDEE"),
                (SyntaxKind::ParameterSeparator, ";"),
                (SyntaxKind::ParameterName, "CN"),
                (SyntaxKind::ParameterEquals, "="),
                (SyntaxKind::QuotedParam, "\"Doe, John\""),
                (SyntaxKind::ParameterSeparator, ";"),
                (SyntaxKind::ParameterName, "ROLE"),
                (SyntaxKind::ParameterEquals, "="),
                (SyntaxKind::BareParam, "REQ"),
                (SyntaxKind::ValueColon, ":"),
                (SyntaxKind::Value, "mailto:a@b.c"),
                (SyntaxKind::LineBreak, "\r\n"),
            ]
        );
    }

    #[test]
    fn multi_value_parameters_split_on_the_comma() {
        assert_eq!(
            kinds_and_text("CATEGORIES;X-Y=a,b;DELEGATED-TO=\"c\",\"d\":Cake, Bread\r\n"),
            vec![
                (SyntaxKind::PropertyName, "CATEGORIES"),
                (SyntaxKind::ParameterSeparator, ";"),
                (SyntaxKind::ParameterName, "X-Y"),
                (SyntaxKind::ParameterEquals, "="),
                (SyntaxKind::BareParam, "a"),
                (SyntaxKind::ValueComma, ","),
                (SyntaxKind::BareParam, "b"),
                (SyntaxKind::ParameterSeparator, ";"),
                (SyntaxKind::ParameterName, "DELEGATED-TO"),
                (SyntaxKind::ParameterEquals, "="),
                (SyntaxKind::QuotedParam, "\"c\""),
                (SyntaxKind::ValueComma, ","),
                (SyntaxKind::QuotedParam, "\"d\""),
                (SyntaxKind::ValueColon, ":"),
                (SyntaxKind::Value, "Cake, Bread"),
                (SyntaxKind::LineBreak, "\r\n"),
            ]
        );
    }

    #[test]
    fn empty_values_contribute_no_token() {
        let lexed = assert_lossless("SUMMARY:\r\n");
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .map(|token| token.kind)
                .collect::<Vec<_>>(),
            vec![
                SyntaxKind::PropertyName,
                SyntaxKind::ValueColon,
                SyntaxKind::LineBreak,
            ]
        );
        assert!(!lexed.has_errors(), "an empty value is well-formed");
        assert_eq!(
            kinds("A;:b\r\n"),
            vec![
                SyntaxKind::PropertyName,
                SyntaxKind::ParameterSeparator,
                SyntaxKind::ValueColon,
                SyntaxKind::Value,
                SyntaxKind::LineBreak,
            ]
        );
        assert_eq!(
            kinds("A;B=:b\r\n"),
            vec![
                SyntaxKind::PropertyName,
                SyntaxKind::ParameterSeparator,
                SyntaxKind::ParameterName,
                SyntaxKind::ParameterEquals,
                SyntaxKind::ValueColon,
                SyntaxKind::Value,
                SyntaxKind::LineBreak,
            ]
        );
        // An empty parameter between two semicolons keeps both delimiters.
        assert_eq!(
            kinds("A;;B=1:x\r\n"),
            vec![
                SyntaxKind::PropertyName,
                SyntaxKind::ParameterSeparator,
                SyntaxKind::ParameterSeparator,
                SyntaxKind::ParameterName,
                SyntaxKind::ParameterEquals,
                SyntaxKind::BareParam,
                SyntaxKind::ValueColon,
                SyntaxKind::Value,
                SyntaxKind::LineBreak,
            ]
        );
    }

    #[test]
    fn a_param_without_a_value_is_a_flagged_run() {
        assert_eq!(
            kinds_and_text("DTSTART;TZID:20260924T090000\r\n"),
            vec![
                (SyntaxKind::PropertyName, "DTSTART"),
                (SyntaxKind::ParameterSeparator, ";"),
                (SyntaxKind::Error, "TZID"),
                (SyntaxKind::ValueColon, ":"),
                (SyntaxKind::Value, "20260924T090000"),
                (SyntaxKind::LineBreak, "\r\n"),
            ]
        );
        assert!(assert_lossless("A;B=1;C:x\r\n").has_errors());
    }

    #[test]
    fn a_fold_is_its_own_token_and_later_spans_stay_line_local() {
        let source = "DESCRIPTION:long line that\r\n continues\r\n";
        let lexed = assert_lossless(source);
        let marker = lexed
            .tokens()
            .iter()
            .find(|token| token.kind == SyntaxKind::FoldMarker)
            .copied()
            .expect("a fold marker");
        assert_eq!(marker.text(source), Some("\r\n "));
        assert_eq!(marker.span, Span::new(26, 29));
        assert!(!marker.has_error());
        for token in lexed.tokens() {
            if token.kind == SyntaxKind::Value {
                assert!(token.span.end <= 26 || token.span.start >= 29, "{source:?}");
            }
        }
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::Value)
                .count(),
            2
        );
        assert!(!lexed.has_errors());
    }

    #[test]
    fn extra_whitespace_after_the_fold_is_still_value_bytes() {
        // Only one whitespace byte is removed by unfolding.
        assert_eq!(
            kinds_and_text("A:1\r\n   two\r\n"),
            vec![
                (SyntaxKind::PropertyName, "A"),
                (SyntaxKind::ValueColon, ":"),
                (SyntaxKind::Value, "1"),
                (SyntaxKind::FoldMarker, "\r\n "),
                (SyntaxKind::Value, "  two"),
                (SyntaxKind::LineBreak, "\r\n"),
            ]
        );
    }

    #[test]
    fn a_fold_with_nothing_before_it_is_still_a_fold_token() {
        let source = "\r\n x";
        let lexed = assert_lossless(source);
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::FoldMarker)
                .count(),
            1,
            "the bytes are a fold; whether anything may be folded into them is \
             the parser's judgement"
        );
        assert!(!lexed.has_errors());
        assert_eq!(
            kinds("A:1\r\n \r\n"),
            vec![
                SyntaxKind::PropertyName,
                SyntaxKind::ValueColon,
                SyntaxKind::Value,
                SyntaxKind::FoldMarker,
                SyntaxKind::LineBreak,
            ]
        );
    }

    #[test]
    fn eof_mid_fold_ends_the_stream_cleanly() {
        for source in ["A:1\r\n ", "A:1\r\n  x", "A:1\r\n", "A:1"] {
            let lexed = assert_lossless(source);
            assert!(!lexed.has_errors(), "{source:?}");
        }
    }

    #[test]
    fn bare_line_endings_are_kept_and_flagged() {
        let lexed = assert_lossless("A:1\nB:2\r\n");
        let breaks: Vec<LexToken> = lexed
            .tokens()
            .iter()
            .copied()
            .filter(|token| token.kind == SyntaxKind::LineBreak)
            .collect();
        assert_eq!(breaks.len(), 2);
        assert_eq!(breaks[0].text(lexed.source()), Some("\n"));
        assert!(breaks[0].has_error(), "RFC 5545 demands CRLF");
        assert_eq!(breaks[1].text(lexed.source()), Some("\r\n"));
        assert!(!breaks[1].has_error());
        for source in ["\r", "\n", "\r\n"] {
            assert_eq!(kinds(source), vec![SyntaxKind::LineBreak]);
        }
    }

    #[test]
    fn a_line_without_a_colon_is_one_flagged_run() {
        let lexed = assert_lossless("no colon here\r\n");
        assert!(lexed.has_errors());
        let flagged: Vec<LexToken> = lexed
            .tokens()
            .iter()
            .copied()
            .filter(|token| token.has_error())
            .collect();
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].kind, SyntaxKind::Error);
        assert_eq!(flagged[0].text(lexed.source()), Some("no colon here"));
        assert_eq!(
            kinds_and_text("SUM X:1\r\n"),
            vec![
                (SyntaxKind::Error, "SUM X:1"),
                (SyntaxKind::LineBreak, "\r\n"),
            ]
        );
    }

    #[test]
    fn lone_delimiters_still_cover_their_byte() {
        for source in [":", ";", "\"", "=", ",", "\r\n \r\n", "   "] {
            let lexed = assert_lossless(source);
            assert!(!lexed.tokens().is_empty(), "{source:?} produced nothing");
            assert_eq!(lexed.joined(), source);
        }
        assert_eq!(kinds_and_text(":"), vec![(SyntaxKind::Error, ":")]);
        assert_eq!(
            kinds_and_text(";"),
            vec![(SyntaxKind::Error, ";")],
            "a line with no name has no structure to report"
        );
    }

    #[test]
    fn lowercase_names_lex_as_names_not_errors() {
        // Case is a conformance question, not a tokenization one: the parser
        // raises the warning, the bytes keep their property-name reading.
        let lexed = assert_lossless("vevent:x\r\n");
        let name = lexed.tokens()[0];
        assert_eq!(name.kind, SyntaxKind::PropertyName);
        assert_eq!(name.text(lexed.source()), Some("vevent"));
        assert!(!name.has_error());
        let begin = assert_lossless("begin:vcalendar\r\n");
        assert_eq!(begin.tokens()[0].kind, SyntaxKind::StructureMarker);
    }

    #[test]
    fn unterminated_quoted_parameter_keeps_its_span_and_flags_it() {
        let source = "A;B=\"oops:x\r\n";
        let lexed = assert_lossless(source);
        let quoted: Vec<LexToken> = lexed
            .tokens()
            .iter()
            .copied()
            .filter(|token| token.kind == SyntaxKind::QuotedParam)
            .collect();
        assert_eq!(quoted.len(), 1);
        assert!(quoted[0].has_error());
        assert_eq!(quoted[0].text(source), Some("\"oops:x"));
        assert_eq!(quoted[0].span, Span::new(4, 11));
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .map(|token| token.kind)
                .collect::<Vec<_>>(),
            vec![
                SyntaxKind::PropertyName,
                SyntaxKind::ParameterSeparator,
                SyntaxKind::ParameterName,
                SyntaxKind::ParameterEquals,
                SyntaxKind::QuotedParam,
                SyntaxKind::LineBreak,
            ]
        );
    }

    #[test]
    fn colon_inside_a_quote_is_not_the_value_delimiter() {
        assert_eq!(
            kinds("A;B=\"x:y\":z\r\n"),
            vec![
                SyntaxKind::PropertyName,
                SyntaxKind::ParameterSeparator,
                SyntaxKind::ParameterName,
                SyntaxKind::ParameterEquals,
                SyntaxKind::QuotedParam,
                SyntaxKind::ValueColon,
                SyntaxKind::Value,
                SyntaxKind::LineBreak,
            ]
        );
    }

    #[test]
    fn text_escapes_are_carved_out_of_the_value() {
        assert_eq!(
            kinds_and_text("DESCRIPTION:a\\nb\\,c\r\n"),
            vec![
                (SyntaxKind::PropertyName, "DESCRIPTION"),
                (SyntaxKind::ValueColon, ":"),
                (SyntaxKind::Value, "a"),
                (SyntaxKind::Escape, "\\n"),
                (SyntaxKind::Value, "b"),
                (SyntaxKind::Escape, "\\,"),
                (SyntaxKind::Value, "c"),
                (SyntaxKind::LineBreak, "\r\n"),
            ]
        );
        // Paired left to right: `\\` is an escaped backslash, so the comma that
        // follows it is ordinary value text.
        assert_eq!(
            kinds_and_text(r#"DESCRIPTION:a\\,b"#),
            vec![
                (SyntaxKind::PropertyName, "DESCRIPTION"),
                (SyntaxKind::ValueColon, ":"),
                (SyntaxKind::Value, "a"),
                (SyntaxKind::Escape, r"\\"),
                (SyntaxKind::Value, ",b"),
            ]
        );
    }

    #[test]
    fn a_backslash_that_opens_nothing_is_kept_and_flagged() {
        let source = "DESCRIPTION:a\\qb\r\n";
        let lexed = assert_lossless(source);
        let escape = lexed
            .tokens()
            .iter()
            .copied()
            .find(|token| token.kind == SyntaxKind::Escape)
            .expect("the stray sequence");
        assert_eq!(escape.text(source), Some("\\q"));
        assert!(escape.has_error());
        // A backslash at the very end of the line has nothing to pair with.
        let trailing = assert_lossless("DESCRIPTION:tail\\\r\n");
        let last = trailing
            .tokens()
            .iter()
            .copied()
            .rev()
            .find(|token| token.kind == SyntaxKind::Escape)
            .expect("the trailing backslash");
        assert_eq!(last.text(trailing.source()), Some("\\"));
        assert!(last.has_error());
    }

    #[test]
    fn non_ascii_never_splits_inside_a_character() {
        assert_eq!(
            kinds_and_text("SUMMARY:名前 😀\r\n"),
            vec![
                (SyntaxKind::PropertyName, "SUMMARY"),
                (SyntaxKind::ValueColon, ":"),
                (SyntaxKind::Value, "名前 😀"),
                (SyntaxKind::LineBreak, "\r\n"),
            ]
        );
        assert_lossless("A;B=\"привет 🚀\":x\r\n");
    }

    #[test]
    fn bom_is_a_single_leading_token() {
        let lexed = assert_lossless("\u{FEFF}BEGIN:VCALENDAR\r\n");
        let first = lexed.tokens()[0];
        assert_eq!(first.kind, SyntaxKind::Bom);
        assert_eq!(first.span, Span::new(0, 3));
        assert_eq!(lexed.tokens()[1].kind, SyntaxKind::StructureMarker);
        assert_eq!(kinds("\u{FEFF}"), vec![SyntaxKind::Bom]);
    }

    #[test]
    fn empty_and_whitespace_only_input_lex_to_little() {
        assert!(assert_lossless("").tokens().is_empty());
        assert_eq!(assert_lossless(" ").tokens().len(), 1);
        assert_eq!(assert_lossless("   ").tokens().len(), 1);
        assert_eq!(assert_lossless("\r\n").tokens().len(), 1);
    }

    #[test]
    fn significant_tokens_drop_only_trivia() {
        let lexed = lex("A:1\r\n 2\r\n");
        assert_eq!(
            lexed
                .significant_tokens()
                .map(|token| token.kind)
                .collect::<Vec<_>>(),
            vec![
                SyntaxKind::PropertyName,
                SyntaxKind::ValueColon,
                SyntaxKind::Value,
                SyntaxKind::Value,
            ]
        );
        assert!(SyntaxKind::FoldMarker.is_trivia());
        assert!(!SyntaxKind::ValueColon.is_trivia());
    }

    #[test]
    fn every_truncation_rebuilds_its_prefix() {
        let sample = concat!(
            "\u{FEFF}BEGIN:VCALENDAR\r\n",
            "SUMMARY:one\r\n two\r\n",
            "ATTENDEE;CN=\"a, b\";X=1:mailto:q\r\n",
            "vevent\n",
            "A;B=\"unterminated\r\n",
            ":\r\n;\r\n",
            "END:VCALENDAR\r\n",
        );
        for cut in 0..=sample.len() {
            if !sample.is_char_boundary(cut) {
                continue;
            }
            assert_lossless(&sample[..cut]);
        }
    }

    #[test]
    fn every_character_boundary_truncation_rebuilds_its_prefix() {
        let sample = "SUMMARY:名前 😀\r\n.cont\r\nA;B=\"привет 🚀\":x\r\n";
        for cut in 0..=sample.len() {
            if !sample.is_char_boundary(cut) {
                continue;
            }
            assert_lossless(&sample[..cut]);
        }
    }

    #[test]
    fn long_and_wide_lines_stay_lossless() {
        let long = format!("DESCRIPTION:{}\r\n", "x".repeat(5000));
        assert_lossless(&long);
        let folded = format!("DESCRIPTION:{}\r\n .\r\n", "y".repeat(2000));
        assert_lossless(&folded);
        let many_params = format!("A;{}:b\r\n", "P=1;".repeat(300));
        assert_lossless(&many_params);
        let many_values = format!("A:{}:b\r\n", ";P=1".repeat(200));
        assert_lossless(&many_values);
    }
}
