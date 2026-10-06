//! A lossless, dialect-parameterised INI/properties lexer.
//!
//! Concatenating token text reconstructs the source byte-for-byte, including
//! for malformed input: an unterminated `[section`, a quoted value that runs
//! into the record break, and a stray run after a closing `"` each keep their
//! own span — flagged [`LexToken::has_error`] — and are never dropped or
//! replaced by a synthesized token. Nothing here borrows a programming-language
//! vocabulary: indentation between a key and its `=` is `padding`, a newline is
//! a `record-break`, and a `\u0041` is an `escape-sequence`.
//!
//! INI and Java `.properties` share this lexer and disagree through [`Options`]
//! alone, on rules that change what the bytes mean:
//!
//! * INI gives `[a]` structure (`section-marker` plus `section-name`), treats
//!   `;` and `#` as comment introducers — including a trailing comment that
//!   whitespace sets off from a value — quotes keys and values with `"`, and
//!   continues a value on the next *indented* line. It has no escapes, so `\n`
//!   in a value is two characters.
//! * `.properties` has no sections at all, so `[a]` is an ordinary key. It
//!   introduces comments with `#` and `!` only (a leading `;` is a key
//!   character), accepts `=`, `:` or a bare whitespace run as the separator,
//!   keeps `\t`, `\uXXXX` and escaped `\= : ! # ` as their own tokens, and
//!   continues a value after a backslash before the record break. Mid-line
//!   `#`, `!` and `;` are value text there, and a `"` is value text too.

use themoretheless_tokenizer_core::Span;

/// Which config-family dialect a document is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Dialect {
    /// Windows-style INI: `[section]` headers over `key = value` entries.
    Ini,
    /// Java `.properties`: one flat key/value namespace, backslash escapes.
    Properties,
}

impl Dialect {
    /// The parameter set this dialect is defined by.
    #[must_use]
    pub const fn options(self) -> Options {
        match self {
            Self::Ini => Options::INI,
            Self::Properties => Options::PROPERTIES,
        }
    }
}

/// Engine options: every axis on which the two dialects part company, exposed
/// individually so a third config dialect can pick its own combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Options {
    /// A leading `[` opens a section header instead of starting a key.
    pub sections: bool,
    /// `;` introduces a comment.
    pub semicolon_comment: bool,
    /// `#` introduces a comment.
    pub hash_comment: bool,
    /// `!` introduces a comment.
    pub bang_comment: bool,
    /// A comment introducer may also end a line's value. Java `.properties`
    /// comments only at line start, so its value text keeps every `#` and `!`.
    pub trailing_comment: bool,
    /// `:` separates key and value as `=` does.
    pub colon_separator: bool,
    /// A whitespace run alone separates key and value.
    pub whitespace_separator: bool,
    /// A `"` at the start of a key or value opens a quoted span.
    pub quoting: bool,
    /// A backslash introduces an escape sequence in key and value text.
    pub escapes: bool,
    /// A backslash before a record break continues the logical line.
    pub backslash_continuation: bool,
    /// An indented physical line continues the previous entry's value.
    pub indented_continuation: bool,
}

impl Options {
    /// INI: sections, `;`/`#` comments, quoted keys and values, `=` only, and
    /// indented continuation lines. No escapes.
    pub const INI: Self = Self {
        sections: true,
        semicolon_comment: true,
        hash_comment: true,
        bang_comment: false,
        colon_separator: false,
        whitespace_separator: false,
        quoting: true,
        escapes: false,
        backslash_continuation: false,
        indented_continuation: true,
        trailing_comment: true,
    };

    /// Java `.properties`: no sections, `#`/`!` comments at line start only,
    /// `=`/`:`/whitespace separators, backslash escapes and continuations. No
    /// quoting and no trailing comment, so a `"` or `#` inside a value is
    /// value text.
    pub const PROPERTIES: Self = Self {
        sections: false,
        semicolon_comment: false,
        hash_comment: true,
        bang_comment: true,
        colon_separator: true,
        whitespace_separator: true,
        quoting: false,
        escapes: true,
        backslash_continuation: true,
        indented_continuation: false,
        trailing_comment: false,
    };
}

impl Default for Options {
    fn default() -> Self {
        Self::INI
    }
}

/// Exact lexical categories emitted by the INI/properties lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxKind {
    /// The UTF-8 encoding of U+FEFF at document start.
    Bom,
    /// One `[` or `]` of a section header.
    SectionMarker,
    /// The bytes between the brackets of a section header.
    SectionName,
    /// Key text, up to the separator or the record break.
    Key,
    /// The separator between key and value: `=`, `:` or a whitespace run.
    Separator,
    /// Value text, up to the record break or a trailing comment.
    Value,
    /// One individual `"` character, opening or closing a key or value.
    Quote,
    /// A comment, from its introducer to the record break.
    Comment,
    /// A `\n`, `\uXXXX` or escaped separator inside key or value text.
    EscapeSequence,
    /// The mechanism that joins physical lines: a trailing `\` with its break,
    /// or the indent that marks an INI continuation line.
    LineContinuation,
    /// Horizontal whitespace that carries no text: indentation, or the run
    /// around a separator.
    Padding,
    /// The `\r\n`, `\n` or `\r` ending a physical line.
    RecordBreak,
    /// A span the lexer flagged but kept: text after a closing quote or after
    /// a closed section header.
    Error,
}

impl SyntaxKind {
    /// Padding, breaks and the BOM carry no key/value structure.
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(self, Self::Bom | Self::Padding | Self::RecordBreak)
    }

    /// Kinds holding the text of a section name, key or value.
    #[must_use]
    pub const fn is_content_text(self) -> bool {
        matches!(
            self,
            Self::SectionName | Self::Key | Self::Value | Self::Quote | Self::EscapeSequence
        )
    }
}

/// Compact per-token state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TokenFlags(u8);

impl TokenFlags {
    pub const EMPTY: Self = Self(0);
    /// The span is part of, or abandoned by, a malformed construct.
    pub const HAS_ERROR: Self = Self(1);
    /// The text sits inside a quoted INI key or value, so its bytes must not
    /// be read as a bare number or boolean.
    pub const QUOTED: Self = Self(2);
    /// On a record break: the next physical line continues this logical one.
    pub const CONTINUED: Self = Self(4);
    /// On an error span: the stray text followed a closed section header
    /// rather than a closing quote.
    pub const AFTER_SECTION: Self = Self(8);
    /// On an escape span: the sequence is not one `.properties` documents.
    pub const INVALID_ESCAPE: Self = Self(16);

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

/// A lexical token. Spans are non-empty UTF-8 byte ranges; a zero-length key,
/// value or section name produces no token at all.
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

    /// Whether this text token was lexed inside a quoted INI span.
    #[must_use]
    pub const fn is_quoted(self) -> bool {
        self.flags.contains(TokenFlags::QUOTED)
    }

    /// On a record break: the following line continues this logical line.
    #[must_use]
    pub const fn continues_line(self) -> bool {
        self.flags.contains(TokenFlags::CONTINUED)
    }

    /// Whether this stray span trailed a section header rather than a quote.
    #[must_use]
    pub const fn after_section_header(self) -> bool {
        self.flags.contains(TokenFlags::AFTER_SECTION)
    }

    /// On an escape span: the sequence is not one the dialect documents.
    #[must_use]
    pub const fn is_invalid_escape(self) -> bool {
        self.flags.contains(TokenFlags::INVALID_ESCAPE)
    }
}

/// Lossless lexer output. Concatenating token text always reconstructs the
/// original source byte-for-byte.
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

    /// Named lossless violation. Streams from [`lex`] always pass; the check
    /// is public so a host can re-verify a transformed token list.
    pub fn verify_lossless(&self) -> Result<(), themoretheless_tokenizer_core::LosslessViolation> {
        themoretheless_tokenizer_core::verify_lossless_spans(
            self.source,
            self.tokens.iter().map(|token| token.span),
        )
    }
}

/// Lexes INI or `.properties` depending on `options`.
#[must_use]
pub fn lex(source: &str, options: Options) -> Lexed<'_> {
    let mut lexer = Lexer {
        bytes: source.as_bytes(),
        pos: 0,
        tokens: Vec::new(),
        options,
        mode: Mode::Entry,
        value_open: false,
        line_closed: false,
    };
    lexer.run();
    Lexed {
        source,
        tokens: lexer.tokens,
    }
}

/// Which region a physical line starts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// A fresh logical line: comment, section header, or key/value entry.
    Entry,
    /// The key region of a line joined by a backslash continuation.
    Key,
    /// The value region of a joined or indented continuation line.
    Value,
}

/// The two text regions of an entry, which stop scanning on different bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Region {
    Key,
    Value,
}

impl Region {
    const fn kind(self) -> SyntaxKind {
        match self {
            Self::Key => SyntaxKind::Key,
            Self::Value => SyntaxKind::Value,
        }
    }

    const fn mode(self) -> Mode {
        match self {
            Self::Key => Mode::Key,
            Self::Value => Mode::Value,
        }
    }
}

struct Lexer<'source> {
    bytes: &'source [u8],
    pos: usize,
    tokens: Vec<LexToken>,
    options: Options,
    mode: Mode,
    /// The previous line left a value open for an indented continuation.
    value_open: bool,
    /// A continuation break already consumed this line's ending.
    line_closed: bool,
}

/// UTF-8 encoding of U+FEFF.
const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

impl Lexer<'_> {
    fn run(&mut self) {
        self.lex_bom();
        while self.pos < self.bytes.len() {
            self.lex_line();
        }
    }

    /// One physical line: optional indentation, one region body, and the
    /// record break that ends it (unless a continuation swallowed that break).
    fn lex_line(&mut self) {
        let resume = self.mode;
        let carried_value = self.value_open;
        self.mode = Mode::Entry;
        self.line_closed = false;
        self.value_open = false;

        let ws_start = self.pos;
        let ws_end = self.whitespace_run(ws_start);
        if ws_end > ws_start {
            let indented = matches!(resume, Mode::Entry)
                && self.options.indented_continuation
                && carried_value
                && !self.at_line_end(ws_end);
            if indented {
                self.mark_break_continued();
                self.push(
                    SyntaxKind::LineContinuation,
                    ws_start,
                    ws_end,
                    TokenFlags::EMPTY,
                );
                self.pos = ws_end;
                // The record break sets this run off, so a comment introducer
                // here starts a comment; `.properties` has no such branch.
                if self.is_comment_byte(self.bytes[self.pos]) {
                    self.lex_comment();
                } else {
                    self.lex_value_region();
                }
                self.close_line();
                return;
            }
            self.push(SyntaxKind::Padding, ws_start, ws_end, TokenFlags::EMPTY);
        }
        self.pos = ws_end;
        if self.at_line_end(self.pos) {
            self.close_line();
            return;
        }
        match resume {
            Mode::Value => self.lex_value_region(),
            Mode::Entry if self.is_comment_byte(self.bytes[self.pos]) => self.lex_comment(),
            Mode::Entry if self.options.sections && self.bytes[self.pos] == b'[' => {
                self.lex_section_header();
            }
            Mode::Entry | Mode::Key => self.lex_entry(),
        }
        self.close_line();
    }

    /// Emit this line's record break, unless a continuation already took it.
    fn close_line(&mut self) {
        if self.line_closed {
            return;
        }
        if self.pos < self.bytes.len() {
            let end = self.break_end(self.pos);
            self.push(SyntaxKind::RecordBreak, self.pos, end, TokenFlags::EMPTY);
            self.pos = end;
        }
    }

    fn lex_bom(&mut self) {
        if self.bytes.starts_with(&BOM) {
            self.push(SyntaxKind::Bom, 0, BOM.len(), TokenFlags::EMPTY);
            self.pos = BOM.len();
        }
    }

    /// Key, separator and value regions of one logical line.
    fn lex_entry(&mut self) {
        self.lex_region(Region::Key);
        if self.line_closed {
            return;
        }
        self.lex_separator();
        if self.line_closed || self.at_line_end(self.pos) {
            return;
        }
        self.lex_value_region();
    }

    fn lex_region(&mut self, region: Region) {
        if region == Region::Key && self.options.quoting && self.peek() == Some(b'"') {
            self.lex_quoted_span(region);
            self.skip_padding();
            return;
        }
        if region == Region::Value && self.options.quoting && self.peek() == Some(b'"') {
            self.lex_quoted_span(region);
            self.skip_padding();
            self.lex_value_tail();
            return;
        }
        if self.options.escapes {
            self.lex_escaped_text(region);
        } else if region == Region::Key {
            self.lex_plain_key();
        } else {
            self.lex_plain_value();
        }
    }

    fn lex_value_region(&mut self) {
        self.lex_region(Region::Value);
    }

    // ─── keys ────────────────────────────────────────────────────────────────

    /// INI key text runs to the `=` or the record break, so interior spaces are
    /// part of the name; only trailing whitespace splits off as padding.
    fn lex_plain_key(&mut self) {
        let start = self.pos;
        let mut end = start;
        while end < self.bytes.len() && !self.at_line_end(end) && self.bytes[end] != b'=' {
            end += 1;
        }
        let mut trimmed = end;
        while trimmed > start && is_space(self.bytes[trimmed - 1]) {
            trimmed -= 1;
        }
        self.push(SyntaxKind::Key, start, trimmed, TokenFlags::EMPTY);
        self.push(SyntaxKind::Padding, trimmed, end, TokenFlags::EMPTY);
        self.pos = end;
    }

    /// `.properties` text: a key ends at an unescaped `=`, `:` or space, a
    /// value only at the record break, and every `\x` pair is its own token.
    fn lex_escaped_text(&mut self, region: Region) {
        let mut run_start = self.pos;
        let mut end = run_start;
        loop {
            if end >= self.bytes.len() || self.at_line_end(end) {
                break;
            }
            let byte = self.bytes[end];
            if byte == b'\\' {
                if self.options.backslash_continuation
                    && end + 1 < self.bytes.len()
                    && self.at_line_end(end + 1)
                {
                    self.push(region.kind(), run_start, end, TokenFlags::EMPTY);
                    self.pos = end;
                    self.lex_continuation_break(region);
                    return;
                }
                self.push(region.kind(), run_start, end, TokenFlags::EMPTY);
                self.pos = end;
                self.lex_escape();
                // The escape ended this run; the next one starts after it.
                end = self.pos;
                run_start = end;
                continue;
            }
            if region == Region::Key && (self.is_separator_byte(byte) || is_space(byte)) {
                break;
            }
            end += 1;
        }
        self.push(region.kind(), run_start, end, TokenFlags::EMPTY);
        self.pos = end;
    }

    fn is_separator_byte(&self, byte: u8) -> bool {
        byte == b'=' || (self.options.colon_separator && byte == b':')
    }

    // ─── separators ──────────────────────────────────────────────────────────

    fn lex_separator(&mut self) {
        if !self.options.whitespace_separator {
            self.skip_padding();
            if self.peek() == Some(b'=') {
                self.push(
                    SyntaxKind::Separator,
                    self.pos,
                    self.pos + 1,
                    TokenFlags::EMPTY,
                );
                self.pos += 1;
                self.skip_padding();
                self.value_open = true;
            }
            return;
        }
        let ws_start = self.pos;
        let ws_end = self.whitespace_run(ws_start);
        let opener = self.bytes.get(ws_end).copied();
        if opener == Some(b'=') || (self.options.colon_separator && opener == Some(b':')) {
            self.push(SyntaxKind::Padding, ws_start, ws_end, TokenFlags::EMPTY);
            self.push(SyntaxKind::Separator, ws_end, ws_end + 1, TokenFlags::EMPTY);
            self.pos = ws_end + 1;
            self.skip_padding();
            self.value_open = true;
            return;
        }
        if ws_end == ws_start {
            return;
        }
        // Whitespace alone separates key from value, but only when something
        // follows it on the line. A dialect with trailing comments stops at a
        // comment introducer there; `.properties` reads it as value text.
        let comment = self.options.trailing_comment && self.is_comment_byte(self.bytes[ws_end]);
        if !self.at_line_end(ws_end) && !comment {
            self.push(SyntaxKind::Separator, ws_start, ws_end, TokenFlags::EMPTY);
            self.pos = ws_end;
            self.value_open = true;
            return;
        }
        self.push(SyntaxKind::Padding, ws_start, ws_end, TokenFlags::EMPTY);
        self.pos = ws_end;
        if !self.at_line_end(self.pos) {
            self.lex_comment();
        }
    }

    // ─── values ──────────────────────────────────────────────────────────────

    /// INI value text runs to the record break, or to a comment introducer
    /// that whitespace sets off from the value.
    fn lex_plain_value(&mut self) {
        let start = self.pos;
        let end = self.line_content_end(start);
        // A comment that ends the value keeps its own preceding whitespace as
        // padding, so the value text never swallows the gap before the `;`.
        let mut trimmed = end;
        while trimmed > start && is_space(self.bytes[trimmed - 1]) {
            trimmed -= 1;
        }
        self.push(SyntaxKind::Value, start, trimmed, TokenFlags::EMPTY);
        self.push(SyntaxKind::Padding, trimmed, end, TokenFlags::EMPTY);
        self.pos = end;
        if end < self.bytes.len() && self.is_comment_byte(self.bytes[end]) {
            self.lex_comment();
        }
        self.value_open = true;
    }

    /// What trails a quoted value: nothing, a comment, or stray text that the
    /// closing quote no longer covers.
    fn lex_value_tail(&mut self) {
        if self.at_line_end(self.pos) {
            return;
        }
        let start = self.pos;
        if self.is_comment_byte(self.bytes[start]) {
            self.lex_comment();
            return;
        }
        let end = self.line_end_from(start);
        self.push(SyntaxKind::Error, start, end, TokenFlags::HAS_ERROR);
        self.pos = end;
    }

    /// End of value text at `start`: the record break, or the first comment
    /// introducer a space or tab sets off from the value. A glued `=;text`
    /// stays value bytes, and a dialect without trailing comments never stops
    /// early at all.
    fn line_content_end(&self, start: usize) -> usize {
        if !self.options.trailing_comment {
            return self.line_end_from(start);
        }
        let mut end = start;
        while end < self.bytes.len() && !self.at_line_end(end) {
            if self.is_comment_byte(self.bytes[end]) && end > 0 && is_space(self.bytes[end - 1]) {
                break;
            }
            end += 1;
        }
        end
    }

    /// A quoted key or value: opener, one content run that never crosses a
    /// record break, and a closer. An unclosed one flags its opener and stops
    /// at the break, so the rest of the document still lexes.
    fn lex_quoted_span(&mut self, region: Region) {
        let opener = self
            .push(SyntaxKind::Quote, self.pos, self.pos + 1, TokenFlags::EMPTY)
            .expect("a one-byte quote always produces a token");
        self.pos += 1;
        let start = self.pos;
        let mut end = start;
        while end < self.bytes.len() && self.bytes[end] != b'"' && !self.at_line_end(end) {
            end += 1;
        }
        self.push(region.kind(), start, end, TokenFlags::QUOTED);
        self.pos = end;
        if self.peek() == Some(b'"') {
            self.push(SyntaxKind::Quote, end, end + 1, TokenFlags::EMPTY);
            self.pos = end + 1;
        } else {
            self.mark_error(opener);
        }
    }

    // ─── comments, sections, escapes ─────────────────────────────────────────

    fn lex_comment(&mut self) {
        let start = self.pos;
        let end = self.line_end_from(start);
        self.push(SyntaxKind::Comment, start, end, TokenFlags::EMPTY);
        self.pos = end;
    }

    fn lex_section_header(&mut self) {
        let opener = self
            .push(
                SyntaxKind::SectionMarker,
                self.pos,
                self.pos + 1,
                TokenFlags::EMPTY,
            )
            .expect("a one-byte bracket always produces a token");
        self.pos += 1;
        let name_start = self.pos;
        let mut end = name_start;
        while end < self.bytes.len() && self.bytes[end] != b']' && !self.at_line_end(end) {
            end += 1;
        }
        self.push(SyntaxKind::SectionName, name_start, end, TokenFlags::EMPTY);
        self.pos = end;
        if self.peek() != Some(b']') {
            self.mark_error(opener);
            return;
        }
        self.push(SyntaxKind::SectionMarker, end, end + 1, TokenFlags::EMPTY);
        self.pos = end + 1;
        self.skip_padding();
        if self.at_line_end(self.pos) {
            return;
        }
        let start = self.pos;
        if self.is_comment_byte(self.bytes[start]) {
            self.lex_comment();
            return;
        }
        let end = self.line_end_from(start);
        self.push(
            SyntaxKind::Error,
            start,
            end,
            TokenFlags::HAS_ERROR.with(TokenFlags::AFTER_SECTION),
        );
        self.pos = end;
    }

    /// One `.properties` escape from the backslash at `pos`.
    fn lex_escape(&mut self) {
        let start = self.pos;
        let mut invalid = false;
        let end = match self.bytes.get(start + 1).copied() {
            None => {
                invalid = true;
                start + 1
            }
            Some(b'u') if self.has_hex_digits(start + 2) => start + 6,
            Some(b'u') => {
                invalid = true;
                start + 2
            }
            Some(next) => {
                invalid = !matches!(
                    next,
                    b't' | b'r' | b'n' | b'f' | b'\\' | b'=' | b':' | b'!' | b'#' | b' '
                );
                start + 2
            }
        };
        let flags = if invalid {
            TokenFlags::INVALID_ESCAPE
        } else {
            TokenFlags::EMPTY
        };
        self.push(SyntaxKind::EscapeSequence, start, end, flags);
        self.pos = end;
    }

    fn has_hex_digits(&self, at: usize) -> bool {
        self.bytes.len() >= at + 4 && self.bytes[at..at + 4].iter().all(u8::is_ascii_hexdigit)
    }

    /// A backslash that takes the record break with it: the logical line goes
    /// on, so no `record-break` token is emitted for this break.
    fn lex_continuation_break(&mut self, region: Region) {
        let start = self.pos;
        let end = self.break_end(start + 1);
        self.push(SyntaxKind::LineContinuation, start, end, TokenFlags::EMPTY);
        self.pos = end;
        self.mode = region.mode();
        self.value_open = false;
        self.line_closed = true;
    }

    // ─── byte helpers ────────────────────────────────────────────────────────

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn is_comment_byte(&self, byte: u8) -> bool {
        byte == b';' && self.options.semicolon_comment
            || byte == b'#' && self.options.hash_comment
            || byte == b'!' && self.options.bang_comment
    }

    /// Whether `at` is a record break or the end of input.
    fn at_line_end(&self, at: usize) -> bool {
        at >= self.bytes.len() || self.is_break(self.bytes[at])
    }

    fn is_break(&self, byte: u8) -> bool {
        byte == b'\n' || byte == b'\r'
    }

    /// End of the record break starting at `start`: `\r\n` is one break.
    fn break_end(&self, start: usize) -> usize {
        let mut end = start + 1;
        if self.bytes[start] == b'\r' && self.bytes.get(end) == Some(&b'\n') {
            end += 1;
        }
        end
    }

    /// End of the line's content at `start`: the record break or end of input.
    fn line_end_from(&self, start: usize) -> usize {
        let mut end = start;
        while end < self.bytes.len() && !self.at_line_end(end) {
            end += 1;
        }
        end
    }

    /// End of the space/tab run starting at `from`.
    fn whitespace_run(&self, from: usize) -> usize {
        let mut end = from;
        while end < self.bytes.len() && is_space(self.bytes[end]) {
            end += 1;
        }
        end
    }

    fn skip_padding(&mut self) {
        let start = self.pos;
        let end = self.whitespace_run(start);
        self.push(SyntaxKind::Padding, start, end, TokenFlags::EMPTY);
        self.pos = end;
    }

    // ─── token output ────────────────────────────────────────────────────────

    /// Emits a token and returns its index. Zero-width spans are skipped so
    /// the stream never carries an empty token: an empty value or a `[]`
    /// header contributes no bytes and no token.
    fn push(
        &mut self,
        kind: SyntaxKind,
        start: usize,
        end: usize,
        flags: TokenFlags,
    ) -> Option<usize> {
        if end <= start {
            return None;
        }
        self.tokens.push(LexToken {
            kind,
            span: Span::new(start, end),
            flags,
        });
        Some(self.tokens.len() - 1)
    }

    fn mark_error(&mut self, index: usize) {
        if let Some(token) = self.tokens.get_mut(index) {
            token.flags = token.flags.with(TokenFlags::HAS_ERROR);
        }
    }

    /// Flags the record break that a following indented line continues.
    fn mark_break_continued(&mut self) {
        let Some(index) = self.tokens.len().checked_sub(1) else {
            return;
        };
        if let Some(token) = self.tokens.get_mut(index)
            && token.kind == SyntaxKind::RecordBreak
        {
            token.flags = token.flags.with(TokenFlags::CONTINUED);
        }
    }
}

/// Horizontal whitespace that separates tokens without ending a line.
fn is_space(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str, options: Options) -> Vec<SyntaxKind> {
        lex(source, options)
            .tokens()
            .iter()
            .map(|token| token.kind)
            .collect()
    }

    fn tagged(source: &str, options: Options) -> Vec<(SyntaxKind, &str)> {
        let lexed = lex(source, options);
        lexed
            .tokens()
            .iter()
            .map(|token| (token.kind, token.text(lexed.source()).unwrap_or_default()))
            .collect()
    }

    pub(crate) fn assert_lossless(source: &str, options: Options) -> Lexed<'_> {
        let lexed = lex(source, options);
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

    #[test]
    fn ini_corpus_is_lossless() {
        let corpus = [
            "[section]\nkey = value\n",
            "; comment\n# comment\n",
            "key=value\nkey2\n",
            "key = \"quoted value\" ; trailing\n",
            "key = one\n    continued\n    and more\n",
            "key = \"unclosed\nnext = 1\n",
            "[unterminated\nkey = 1\n",
            "[a]junk\n",
            "empty =\n",
            "empty_quoted = \"\"\n",
            "\u{FEFF}[s]\nk=v\n",
            "path = C:\\temp;x\n",
            "trailing=break\r\n",
            "crlf = 1\r\nnext = 2\r\n",
            "",
            "\n",
            "[]\n",
            "[ ]\n",
            "= orphan\n",
            "no newline at eof",
            "名前 = 真美\n",
            "a\n\nb\n",
            "key = \"a\" junk here\n",
            "!bang = not a comment\n",
        ];
        for source in corpus {
            assert_lossless(source, Options::INI);
        }
    }

    #[test]
    fn properties_corpus_is_lossless() {
        let corpus = [
            "key=value\nkey2:value\nkey3 value\n",
            "# comment\n! comment\n; not a comment\n",
            "a\\=b = escaped separator\n",
            "key = line one\\\n    line two\n",
            "unicode = \\u0041\\u00e9\n",
            "tabs = a\\tb\\nc\\\\\n",
            "bad = \\q and \\u12g4\n",
            "trailing backslash \\",
            "[a] = ordinary key\n",
            "empty =\n",
            "quoted = \"literal\"\n",
            "\u{FEFF}a=b\n",
            "",
            "\n",
            "key = value ! not a comment\n",
            "joined\\\nkey = 1\n",
            "crlf = 1\r\nnext = 2\r\n",
            "名前 = 真美\n",
            "a\n\nb\n",
            "no newline at eof",
        ];
        for source in corpus {
            assert_lossless(source, Options::PROPERTIES);
        }
    }

    #[test]
    fn every_truncation_rebuilds_its_prefix() {
        let ini = "[s]\nkey = \"a;b\" ; c\ncont = one\n  two\n[unterminated\nplain\n";
        for cut in 0..=ini.len() {
            assert_lossless(&ini[..cut], Options::INI);
        }
        let properties = "a\\:b = x\\u0041y\\\n  cont\ntab = 2\nbad=\\q\n";
        for cut in 0..=properties.len() {
            assert_lossless(&properties[..cut], Options::PROPERTIES);
        }
    }

    #[test]
    fn ini_section_header_splits_into_marker_name_marker() {
        assert_eq!(
            tagged("[network]\n", Options::INI),
            vec![
                (SyntaxKind::SectionMarker, "["),
                (SyntaxKind::SectionName, "network"),
                (SyntaxKind::SectionMarker, "]"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
        assert_eq!(
            tagged("  [ s ]  ; note\n", Options::INI),
            vec![
                (SyntaxKind::Padding, "  "),
                (SyntaxKind::SectionMarker, "["),
                (SyntaxKind::SectionName, " s "),
                (SyntaxKind::SectionMarker, "]"),
                (SyntaxKind::Padding, "  "),
                (SyntaxKind::Comment, "; note"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
    }

    #[test]
    fn ini_entry_kinds_are_exact() {
        assert_eq!(
            tagged("key = value\n", Options::INI),
            vec![
                (SyntaxKind::Key, "key"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "value"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
        assert_eq!(
            tagged("key\n", Options::INI),
            vec![(SyntaxKind::Key, "key"), (SyntaxKind::RecordBreak, "\n")]
        );
        assert_eq!(
            tagged("empty =\n", Options::INI),
            vec![
                (SyntaxKind::Key, "empty"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::RecordBreak, "\n"),
            ],
            "an empty value carries no token"
        );
    }

    #[test]
    fn ini_keeps_interior_whitespace_in_keys() {
        assert_eq!(
            tagged("my key = 1\n", Options::INI),
            vec![
                (SyntaxKind::Key, "my key"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "1"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
        assert_eq!(
            tagged("a b\n", Options::INI),
            vec![(SyntaxKind::Key, "a b"), (SyntaxKind::RecordBreak, "\n")],
            "whitespace is never an INI separator"
        );
    }

    #[test]
    fn ini_quoted_key_and_value_are_flagged_not_reparsed() {
        assert_eq!(
            tagged("\"my key\" = \"a value\"\n", Options::INI),
            vec![
                (SyntaxKind::Quote, "\""),
                (SyntaxKind::Key, "my key"),
                (SyntaxKind::Quote, "\""),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Quote, "\""),
                (SyntaxKind::Value, "a value"),
                (SyntaxKind::Quote, "\""),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
        let lexed = assert_lossless("k = \"v\"\n", Options::INI);
        let quoted = lexed
            .tokens()
            .iter()
            .find(|token| token.kind == SyntaxKind::Value)
            .copied()
            .expect("the value");
        assert!(quoted.is_quoted());
        assert!(!quoted.has_error());
        assert_eq!(quoted.flags, TokenFlags::QUOTED);
        assert!(TokenFlags::EMPTY.is_empty());
        assert!(!TokenFlags::QUOTED.is_empty());
    }

    #[test]
    fn ini_comment_introducers_and_their_boundaries() {
        for (source, expected) in [
            (
                ";full line\n",
                vec![
                    (SyntaxKind::Comment, ";full line"),
                    (SyntaxKind::RecordBreak, "\n"),
                ],
            ),
            (
                "#hash\n",
                vec![
                    (SyntaxKind::Comment, "#hash"),
                    (SyntaxKind::RecordBreak, "\n"),
                ],
            ),
            (
                "k = v ; trailing\n",
                vec![
                    (SyntaxKind::Key, "k"),
                    (SyntaxKind::Padding, " "),
                    (SyntaxKind::Separator, "="),
                    (SyntaxKind::Padding, " "),
                    (SyntaxKind::Value, "v"),
                    (SyntaxKind::Padding, " "),
                    (SyntaxKind::Comment, "; trailing"),
                    (SyntaxKind::RecordBreak, "\n"),
                ],
            ),
            (
                "k = v;not-a-comment\n",
                vec![
                    (SyntaxKind::Key, "k"),
                    (SyntaxKind::Padding, " "),
                    (SyntaxKind::Separator, "="),
                    (SyntaxKind::Padding, " "),
                    (SyntaxKind::Value, "v;not-a-comment"),
                    (SyntaxKind::RecordBreak, "\n"),
                ],
            ),
            (
                "k = v\t# tab-set-off\n",
                vec![
                    (SyntaxKind::Key, "k"),
                    (SyntaxKind::Padding, " "),
                    (SyntaxKind::Separator, "="),
                    (SyntaxKind::Padding, " "),
                    (SyntaxKind::Value, "v"),
                    (SyntaxKind::Padding, "\t"),
                    (SyntaxKind::Comment, "# tab-set-off"),
                    (SyntaxKind::RecordBreak, "\n"),
                ],
            ),
            (
                "k =;glued\n",
                vec![
                    (SyntaxKind::Key, "k"),
                    (SyntaxKind::Padding, " "),
                    (SyntaxKind::Separator, "="),
                    (SyntaxKind::Value, ";glued"),
                    (SyntaxKind::RecordBreak, "\n"),
                ],
            ),
        ] {
            assert_eq!(tagged(source, Options::INI), expected, "{source:?}");
        }
    }

    #[test]
    fn ini_bang_is_a_key_not_a_comment() {
        assert_eq!(
            tagged("!important = yes\n", Options::INI),
            vec![
                (SyntaxKind::Key, "!important"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "yes"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
    }

    #[test]
    fn ini_indented_continuation_marks_both_mechanism_and_break() {
        let source = "k = one\n  two\n";
        let lexed = assert_lossless(source, Options::INI);
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .map(|token| (token.kind, token.text(lexed.source()).unwrap_or_default()))
                .collect::<Vec<_>>(),
            vec![
                (SyntaxKind::Key, "k"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "one"),
                (SyntaxKind::RecordBreak, "\n"),
                (SyntaxKind::LineContinuation, "  "),
                (SyntaxKind::Value, "two"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
        let continued: Vec<Span> = lexed
            .tokens()
            .iter()
            .filter(|token| token.kind == SyntaxKind::RecordBreak && token.continues_line())
            .map(|token| token.span)
            .collect();
        assert_eq!(continued, vec![Span::new(7, 8)]);
    }

    #[test]
    fn ini_continuation_needs_indentation_and_a_previous_value() {
        // A section header, a comment line, a blank line and a key without a
        // value all close the previous entry's value.
        for source in [
            "k = one\n[two]\n",
            "k = one\n; c\n",
            "k = one\n\n  two\n",
            "key\n  value\n",
        ] {
            assert!(
                !lex(source, Options::INI)
                    .tokens()
                    .iter()
                    .any(|token| token.kind == SyntaxKind::LineContinuation),
                "{source:?} must not continue"
            );
        }
    }

    #[test]
    fn ini_backslash_is_ordinary_value_text() {
        let lexed = assert_lossless("path = C:\\temp\\n\n", Options::INI);
        let value = lexed
            .tokens()
            .iter()
            .find(|token| token.kind == SyntaxKind::Value)
            .copied()
            .expect("the value");
        assert_eq!(value.text(lexed.source()), Some("C:\\temp\\n"));
        assert!(
            !lexed
                .tokens()
                .iter()
                .any(|token| token.kind == SyntaxKind::EscapeSequence)
        );
    }

    #[test]
    fn properties_comments_use_hash_and_bang_only() {
        assert_eq!(
            tagged("!c\n", Options::PROPERTIES),
            vec![(SyntaxKind::Comment, "!c"), (SyntaxKind::RecordBreak, "\n"),]
        );
        assert_eq!(
            tagged(";k = v\n", Options::PROPERTIES),
            vec![
                (SyntaxKind::Key, ";k"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "v"),
                (SyntaxKind::RecordBreak, "\n"),
            ],
            "a leading semicolon is a key character in .properties"
        );
    }

    #[test]
    fn properties_has_no_sections_at_all() {
        assert_eq!(
            tagged("[a]\n", Options::PROPERTIES),
            vec![(SyntaxKind::Key, "[a]"), (SyntaxKind::RecordBreak, "\n")]
        );
        assert_eq!(
            kinds("[a]\n", Options::PROPERTIES),
            vec![SyntaxKind::Key, SyntaxKind::RecordBreak]
        );
    }

    #[test]
    fn properties_accepts_three_separator_shapes() {
        for (source, separator) in [("k=v\n", "="), ("k:v\n", ":"), ("k v\n", " ")] {
            let lexed = assert_lossless(source, Options::PROPERTIES);
            let sep = lexed
                .tokens()
                .iter()
                .find(|token| token.kind == SyntaxKind::Separator)
                .copied()
                .expect("a separator");
            assert_eq!(sep.text(lexed.source()), Some(separator), "{source:?}");
        }
        assert_eq!(
            tagged("k : v\n", Options::PROPERTIES),
            vec![
                (SyntaxKind::Key, "k"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, ":"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "v"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
        assert_eq!(
            tagged("k  v\n", Options::PROPERTIES),
            vec![
                (SyntaxKind::Key, "k"),
                (SyntaxKind::Separator, "  "),
                (SyntaxKind::Value, "v"),
                (SyntaxKind::RecordBreak, "\n"),
            ],
            "the whitespace run *is* the separator"
        );
        assert_eq!(
            tagged("k = v w\n", Options::PROPERTIES),
            vec![
                (SyntaxKind::Key, "k"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "v w"),
                (SyntaxKind::RecordBreak, "\n"),
            ],
            "whitespace separates once, then it is value text"
        );
    }

    #[test]
    fn properties_has_no_inline_comments_and_no_trailing_comment() {
        assert_eq!(
            tagged("k = v # note\n", Options::PROPERTIES),
            vec![
                (SyntaxKind::Key, "k"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "v # note"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
        assert_eq!(
            tagged("k = v ! note\n", Options::PROPERTIES),
            vec![
                (SyntaxKind::Key, "k"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "v ! note"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
    }

    #[test]
    fn url_values_keep_their_colons_and_semicolons() {
        assert_eq!(
            tagged("url = http://example.com:80/a;b#c\n", Options::PROPERTIES),
            vec![
                (SyntaxKind::Key, "url"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "http://example.com:80/a;b#c"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
    }

    #[test]
    fn every_documented_escape_is_its_own_token() {
        let source = "k = a\\tb\\nc\\\\d\\=e\\:f\\!g\\#h\\u0041\\u00e9\n";
        let lexed = assert_lossless(source, Options::PROPERTIES);
        let escapes = lexed
            .tokens()
            .iter()
            .filter(|token| token.kind == SyntaxKind::EscapeSequence)
            .map(|token| token.text(lexed.source()).unwrap_or_default())
            .collect::<Vec<_>>();
        assert_eq!(
            escapes,
            vec![
                "\\t", "\\n", "\\\\", "\\=", "\\:", "\\!", "\\#", "\\u0041", "\\u00e9"
            ]
        );
        assert!(
            !lexed
                .tokens()
                .iter()
                .any(|token| token.kind == SyntaxKind::EscapeSequence && token.is_invalid_escape()),
            "every escape here is documented"
        );
    }

    #[test]
    fn escaped_separators_stay_inside_the_key() {
        assert_eq!(
            tagged("a\\=b\\:c\\!d\\#e\\ f = v\n", Options::PROPERTIES),
            vec![
                (SyntaxKind::Key, "a"),
                (SyntaxKind::EscapeSequence, "\\="),
                (SyntaxKind::Key, "b"),
                (SyntaxKind::EscapeSequence, "\\:"),
                (SyntaxKind::Key, "c"),
                (SyntaxKind::EscapeSequence, "\\!"),
                (SyntaxKind::Key, "d"),
                (SyntaxKind::EscapeSequence, "\\#"),
                (SyntaxKind::Key, "e"),
                (SyntaxKind::EscapeSequence, "\\ "),
                (SyntaxKind::Key, "f"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "v"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
    }

    #[test]
    fn backslash_continuation_takes_the_break_with_it() {
        let source = "k = one\\\n  two\n";
        let lexed = assert_lossless(source, Options::PROPERTIES);
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .map(|token| (token.kind, token.text(lexed.source()).unwrap_or_default()))
                .collect::<Vec<_>>(),
            vec![
                (SyntaxKind::Key, "k"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "one"),
                (SyntaxKind::LineContinuation, "\\\n"),
                (SyntaxKind::Padding, "  "),
                (SyntaxKind::Value, "two"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::RecordBreak)
                .count(),
            1,
            "the continued break belongs to the line-continuation token"
        );
    }

    #[test]
    fn crlf_continuation_is_one_line_continuation_token() {
        let lexed = assert_lossless("k = a\\\r\nb\n", Options::PROPERTIES);
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::LineContinuation)
                .map(|token| token.span)
                .collect::<Vec<_>>(),
            vec![Span::new(5, 8)]
        );
    }

    #[test]
    fn a_continued_key_region_resumes_after_the_break() {
        assert_eq!(
            tagged("k\\\n= v\n", Options::PROPERTIES),
            vec![
                (SyntaxKind::Key, "k"),
                (SyntaxKind::LineContinuation, "\\\n"),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "v"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
    }

    #[test]
    fn unknown_escape_keeps_its_span_and_warns() {
        let lexed = assert_lossless("k = \\q\n", Options::PROPERTIES);
        let escape = lexed
            .tokens()
            .iter()
            .find(|token| token.kind == SyntaxKind::EscapeSequence)
            .copied()
            .expect("the escape");
        assert!(escape.is_invalid_escape());
        assert!(!escape.has_error(), "an unknown escape is a warning");
        assert_eq!(escape.text(lexed.source()), Some("\\q"));
    }

    #[test]
    fn short_unicode_escape_is_flagged_as_invalid() {
        let lexed = assert_lossless("k = \\u12g4\n", Options::PROPERTIES);
        let escape = lexed
            .tokens()
            .iter()
            .find(|token| token.kind == SyntaxKind::EscapeSequence)
            .copied()
            .expect("the escape");
        assert!(escape.is_invalid_escape());
        assert_eq!(escape.text(lexed.source()), Some("\\u"));
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::Value)
                .map(|token| token.text(lexed.source()).unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["12g4"]
        );
    }

    #[test]
    fn trailing_backslash_at_eof_is_an_invalid_escape() {
        let lexed = assert_lossless("k = \\", Options::PROPERTIES);
        let escape = lexed.tokens().last().copied().expect("the escape");
        assert_eq!(escape.kind, SyntaxKind::EscapeSequence);
        assert!(escape.is_invalid_escape());
        assert_eq!(escape.span, Span::new(4, 5));
    }

    #[test]
    fn properties_quoting_is_off_so_a_quote_is_value_text() {
        assert_eq!(
            tagged("k = \"a # b\"\n", Options::PROPERTIES),
            vec![
                (SyntaxKind::Key, "k"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "\"a # b\""),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
        assert!(
            !lex("k = \"a\"\n", Options::PROPERTIES)
                .tokens()
                .iter()
                .any(|token| token.kind == SyntaxKind::Quote)
        );
    }

    #[test]
    fn properties_indentation_never_continues_a_value() {
        assert_eq!(
            tagged("k = one\n  two\n", Options::PROPERTIES),
            vec![
                (SyntaxKind::Key, "k"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "one"),
                (SyntaxKind::RecordBreak, "\n"),
                (SyntaxKind::Padding, "  "),
                (SyntaxKind::Key, "two"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
    }

    #[test]
    fn leading_whitespace_is_stripped_before_a_properties_key() {
        assert_eq!(
            tagged("   k = v\n", Options::PROPERTIES),
            vec![
                (SyntaxKind::Padding, "   "),
                (SyntaxKind::Key, "k"),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Padding, " "),
                (SyntaxKind::Value, "v"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
    }

    #[test]
    fn bom_is_a_single_leading_token() {
        let lexed = assert_lossless("\u{FEFF}[s]\nk=v\n", Options::INI);
        let first = lexed.tokens()[0];
        assert_eq!(first.kind, SyntaxKind::Bom);
        assert_eq!(first.span, Span::new(0, 3));
        assert_eq!(lexed.tokens()[1].kind, SyntaxKind::SectionMarker);
        assert_eq!(kinds("\u{FEFF}", Options::INI), vec![SyntaxKind::Bom]);
        assert_eq!(
            tagged("\u{FEFF}a=b\n", Options::PROPERTIES)[0],
            (SyntaxKind::Bom, "\u{FEFF}")
        );
    }

    #[test]
    fn broken_constructs_are_flagged_not_dropped() {
        let unterminated = assert_lossless("[oops\nk = v\n", Options::INI);
        assert!(unterminated.has_errors());
        assert!(
            unterminated
                .tokens()
                .iter()
                .any(|token| token.kind == SyntaxKind::SectionMarker && token.has_error())
        );
        let unclosed = assert_lossless("k = \"abc\nnext = 1\n", Options::INI);
        assert!(unclosed.has_errors());
        assert_eq!(
            unclosed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::RecordBreak)
                .count(),
            2,
            "recovery must resume on the next line"
        );
        let stray_value = assert_lossless("k = \"v\" junk\n", Options::INI);
        assert!(
            stray_value
                .tokens()
                .iter()
                .any(|token| token.kind == SyntaxKind::Error && !token.after_section_header())
        );
        let stray_section = assert_lossless("[a] junk\n", Options::INI);
        assert!(
            stray_section
                .tokens()
                .iter()
                .any(|token| token.kind == SyntaxKind::Error && token.after_section_header())
        );
        assert!(
            !lex("k = v\n", Options::INI)
                .tokens()
                .iter()
                .any(|token| token.has_error())
        );
    }

    #[test]
    fn every_break_shape_is_one_record_break_token() {
        for line_ending in ["\n", "\r\n", "\r"] {
            for options in [Options::INI, Options::PROPERTIES] {
                let source = format!("k = v{line_ending}next = 2");
                let lexed = assert_lossless(&source, options);
                assert_eq!(
                    lexed
                        .tokens()
                        .iter()
                        .filter(|token| token.kind == SyntaxKind::RecordBreak)
                        .count(),
                    1,
                    "{source:?} {options:?}"
                );
            }
        }
    }

    #[test]
    fn unicode_keys_and_values_stay_whole() {
        let lexed = assert_lossless("ключ = значение\n", Options::INI);
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::Value)
                .map(|token| token.text(lexed.source()).unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["значение"]
        );
        assert_lossless("😀 = 😀😀\n", Options::PROPERTIES);
        assert_lossless("[セクション]\n", Options::INI);
        assert_lossless("k = \\u00e9 and é\n", Options::PROPERTIES);
    }

    #[test]
    fn trivia_and_content_predicates_agree_with_the_vocabulary() {
        for kind in [
            SyntaxKind::Bom,
            SyntaxKind::Padding,
            SyntaxKind::RecordBreak,
        ] {
            assert!(kind.is_trivia(), "{kind:?}");
            assert!(!kind.is_content_text(), "{kind:?}");
        }
        for kind in [
            SyntaxKind::SectionName,
            SyntaxKind::Key,
            SyntaxKind::Value,
            SyntaxKind::Quote,
            SyntaxKind::EscapeSequence,
        ] {
            assert!(kind.is_content_text(), "{kind:?}");
        }
        for kind in [
            SyntaxKind::SectionMarker,
            SyntaxKind::Separator,
            SyntaxKind::Comment,
            SyntaxKind::LineContinuation,
            SyntaxKind::Error,
        ] {
            assert!(!kind.is_trivia() && !kind.is_content_text(), "{kind:?}");
        }
        assert_eq!(lex("k = v\n", Options::INI).significant_tokens().count(), 3);
    }

    #[test]
    fn dialect_flags_are_the_only_difference_between_the_two() {
        assert_eq!(Dialect::Ini.options(), Options::INI);
        assert_eq!(Dialect::Properties.options(), Options::PROPERTIES);
        assert_eq!(Options::default(), Options::INI);
        let axes = [
            Options::INI.sections,
            Options::INI.semicolon_comment,
            Options::INI.bang_comment,
            Options::INI.colon_separator,
            Options::INI.whitespace_separator,
            Options::INI.quoting,
            Options::INI.escapes,
            Options::INI.backslash_continuation,
            Options::INI.indented_continuation,
            Options::INI.trailing_comment,
        ];
        let flipped = [
            Options::PROPERTIES.sections,
            Options::PROPERTIES.semicolon_comment,
            Options::PROPERTIES.bang_comment,
            Options::PROPERTIES.colon_separator,
            Options::PROPERTIES.whitespace_separator,
            Options::PROPERTIES.quoting,
            Options::PROPERTIES.escapes,
            Options::PROPERTIES.backslash_continuation,
            Options::PROPERTIES.indented_continuation,
            Options::PROPERTIES.trailing_comment,
        ];
        assert_eq!(axes.len(), flipped.len());
        for (ini, properties) in axes.iter().zip(flipped) {
            assert_ne!(*ini, properties, "every axis must differ");
        }
        assert_eq!(
            Options::INI.hash_comment,
            Options::PROPERTIES.hash_comment,
            "only `#` is a comment in both dialects"
        );
    }
}
