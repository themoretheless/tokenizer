//! A lossless HCL lexer.
//!
//! Concatenating token text reconstructs the source byte-for-byte, including
//! for malformed input: an unterminated string, a heredoc whose terminator
//! never arrives, or a stray byte all keep their own span — flagged
//! [`LexToken::has_error`] — and are never dropped or synthesized. Every
//! advance guarantees progress, so no input can hang or panic the lexer.
//!
//! The vocabulary is HCL's own: line (`#`, `//`) and block (`/* */`)
//! comments, identifiers with `-` and `_`, template strings whose `${...}`
//! interpolations and `\n` / `\uXXXX` / `\xNN` escapes are carved out as
//! their own tokens, heredocs (`<<TAG`, `<<-TAG`) split into an opening
//! marker, a body and a closing marker, `true` / `false` / `null` literal
//! kinds, and newline tokens — HCL delimits items by newlines, so the lexer
//! keeps them distinguishable from ordinary spaces.

use themoretheless_tokenizer_core::{LosslessViolation, Span, verify_lossless_spans};

/// Exact lexical categories emitted by the HCL lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxKind {
    /// The UTF-8 encoding of U+FEFF at document start.
    Bom,
    /// A `#` or `//` comment through the end of its line (newline excluded).
    LineComment,
    /// A `/* ... */` comment.
    BlockComment,
    /// A run of spaces and tabs.
    Whitespace,
    /// A `\n`, `\r\n` or `\r`: HCL's item separator.
    Newline,
    /// A name: ASCII letter or `_` start, then letters, digits, `-`, `_`.
    Identifier,
    /// An integer or float literal, e.g. `8080`, `0.25`, `1.5e3`.
    Number,
    /// The `true` / `false` literals.
    Boolean,
    /// The `null` literal.
    Null,
    /// Literal text of a quoted string, the surrounding quotes included.
    String,
    /// A `\n`, `\xNN`, `\uXXXX`, `\"`, `\\` sequence inside a string.
    Escape,
    /// A complete `${...}` interpolation inside a string or heredoc.
    Interpolation,
    /// The `<<TAG` or `<<-TAG` that opens a heredoc.
    HeredocOpen,
    /// Raw bytes between a heredoc marker line and its terminator line.
    HeredocBody,
    /// The terminator tag text closing a heredoc.
    HeredocClose,
    /// An operator: `= == != ! && || + - * / % ? :`.
    Operator,
    /// Structural punctuation: `{ } [ ] ( ) , .`.
    Punctuation,
    /// A span the lexer flagged but kept: an unknown byte or a lone `$`.
    Error,
}

impl SyntaxKind {
    /// Trivia separates items without carrying HCL content.
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::Bom | Self::Whitespace | Self::Newline | Self::LineComment | Self::BlockComment
        )
    }

    /// Kinds that carry string or heredoc text.
    #[must_use]
    pub const fn is_string_content(self) -> bool {
        matches!(self, Self::String | Self::Escape | Self::Interpolation)
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

/// A lexical token. Spans are non-empty UTF-8 byte ranges; the lexer never
/// emits a zero-width token.
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
    pub fn verify_lossless(&self) -> Result<(), LosslessViolation> {
        verify_lossless_spans(self.source, self.tokens.iter().map(|token| token.span))
    }
}

/// Lexes an HCL document.
#[must_use]
pub fn lex(source: &str) -> Lexed<'_> {
    let mut lexer = Lexer {
        bytes: source.as_bytes(),
        pos: 0,
        tokens: Vec::new(),
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
}

/// UTF-8 encoding of U+FEFF.
const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

const fn is_space(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

const fn is_newline_start(byte: u8) -> bool {
    byte == b'\n' || byte == b'\r'
}

const fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

const fn is_ident_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

const fn is_hex(byte: u8) -> bool {
    byte.is_ascii_hexdigit()
}

impl Lexer<'_> {
    fn run(&mut self) {
        self.lex_bom();
        while self.pos < self.bytes.len() {
            let byte = self.bytes[self.pos];
            if is_space(byte) {
                self.lex_whitespace();
            } else if is_newline_start(byte) {
                let end = self.newline_end(self.pos);
                self.push(SyntaxKind::Newline, self.pos, end, TokenFlags::EMPTY);
                self.pos = end;
            } else if byte == b'#' || (byte == b'/' && self.peek(self.pos + 1) == Some(b'/')) {
                self.lex_line_comment();
            } else if byte == b'/' && self.peek(self.pos + 1) == Some(b'*') {
                self.lex_block_comment();
            } else if is_ident_start(byte) {
                self.lex_identifier();
            } else if byte.is_ascii_digit() {
                self.lex_number();
            } else if byte == b'"' {
                self.lex_string();
            } else if byte == b'<' && self.peek(self.pos + 1) == Some(b'<') {
                self.lex_heredoc();
            } else {
                self.lex_symbol();
            }
        }
    }

    fn lex_bom(&mut self) {
        if self.bytes.starts_with(&BOM) {
            self.push(SyntaxKind::Bom, 0, BOM.len(), TokenFlags::EMPTY);
            self.pos = BOM.len();
        }
    }

    fn peek(&self, index: usize) -> Option<u8> {
        self.bytes.get(index).copied()
    }

    /// End of the newline starting at `start`: `\r\n` is one break.
    fn newline_end(&self, start: usize) -> usize {
        let mut end = start + 1;
        if self.bytes[start] == b'\r' && self.peek(end) == Some(b'\n') {
            end += 1;
        }
        end
    }

    fn lex_whitespace(&mut self) {
        let start = self.pos;
        let mut end = start;
        while end < self.bytes.len() && is_space(self.bytes[end]) {
            end += 1;
        }
        self.push(SyntaxKind::Whitespace, start, end, TokenFlags::EMPTY);
        self.pos = end;
    }

    /// `#` or `//` through the end of the line; the newline itself is its own
    /// token so item separation survives comment stripping.
    fn lex_line_comment(&mut self) {
        let start = self.pos;
        let mut end = start + 1;
        while end < self.bytes.len() && !is_newline_start(self.bytes[end]) {
            end += 1;
        }
        self.push(SyntaxKind::LineComment, start, end, TokenFlags::EMPTY);
        self.pos = end;
    }

    fn lex_block_comment(&mut self) {
        let start = self.pos;
        let mut cursor = start + 2;
        while cursor < self.bytes.len() {
            if self.bytes[cursor] == b'*' && self.peek(cursor + 1) == Some(b'/') {
                cursor += 2;
                self.push(SyntaxKind::BlockComment, start, cursor, TokenFlags::EMPTY);
                self.pos = cursor;
                return;
            }
            cursor += 1;
        }
        self.push(
            SyntaxKind::BlockComment,
            start,
            self.bytes.len(),
            TokenFlags::HAS_ERROR,
        );
        self.pos = self.bytes.len();
    }

    fn lex_identifier(&mut self) {
        let start = self.pos;
        let mut end = start + 1;
        while end < self.bytes.len() && is_ident_continue(self.bytes[end]) {
            end += 1;
        }
        let text = &self.bytes[start..end];
        let kind = if text == b"true" || text == b"false" {
            SyntaxKind::Boolean
        } else if text == b"null" {
            SyntaxKind::Null
        } else {
            SyntaxKind::Identifier
        };
        self.push(kind, start, end, TokenFlags::EMPTY);
        self.pos = end;
    }

    /// Integer, optional fraction, optional exponent — one token each, so
    /// `1.5e-3` never splits.
    fn lex_number(&mut self) {
        let start = self.pos;
        let mut end = start;
        while end < self.bytes.len() && self.bytes[end].is_ascii_digit() {
            end += 1;
        }
        if self.peek(end) == Some(b'.') && self.peek(end + 1).is_some_and(|b| b.is_ascii_digit()) {
            end += 1;
            while end < self.bytes.len() && self.bytes[end].is_ascii_digit() {
                end += 1;
            }
        }
        if matches!(self.peek(end), Some(b'e') | Some(b'E')) {
            let mut exp = end + 1;
            if matches!(self.peek(exp), Some(b'+') | Some(b'-')) {
                exp += 1;
            }
            if self.peek(exp).is_some_and(|b| b.is_ascii_digit()) {
                while exp < self.bytes.len() && self.bytes[exp].is_ascii_digit() {
                    exp += 1;
                }
                end = exp;
            }
        }
        self.push(SyntaxKind::Number, start, end, TokenFlags::EMPTY);
        self.pos = end;
    }

    /// A template string: literal runs interleaved with escapes and `${...}`
    /// interpolations; the surrounding quotes ride on the literal runs so no
    /// byte is lost. An unterminated string flags its final run.
    fn lex_string(&mut self) {
        let open = self.pos;
        let mut run_start = open;
        let mut cursor = open + 1;
        let closed = loop {
            if cursor >= self.bytes.len() {
                break false;
            }
            let byte = self.bytes[cursor];
            match byte {
                b'"' => {
                    self.push(SyntaxKind::String, run_start, cursor + 1, TokenFlags::EMPTY);
                    break true;
                }
                b'\\' => {
                    self.push(SyntaxKind::String, run_start, cursor, TokenFlags::EMPTY);
                    let (esc_end, esc_flags) = self.escape_extent(cursor);
                    self.push(SyntaxKind::Escape, cursor, esc_end, esc_flags);
                    cursor = esc_end;
                    run_start = esc_end;
                }
                b'$' if self.peek(cursor + 1) == Some(b'{') => {
                    self.push(SyntaxKind::String, run_start, cursor, TokenFlags::EMPTY);
                    let interp_end = interpolation_end(self.bytes, cursor + 2);
                    let flags = if interp_end.is_some() {
                        TokenFlags::EMPTY
                    } else {
                        TokenFlags::HAS_ERROR
                    };
                    let end = interp_end.unwrap_or(self.bytes.len());
                    self.push(SyntaxKind::Interpolation, cursor, end, flags);
                    cursor = end;
                    run_start = end;
                }
                _ => cursor += 1,
            }
        };
        if !closed {
            self.push(
                SyntaxKind::String,
                run_start,
                self.bytes.len(),
                TokenFlags::HAS_ERROR,
            );
            self.pos = self.bytes.len();
            return;
        }
        // `cursor` rests on the closing quote; it is already inside the last
        // run, so the scan resumes one byte past it.
        self.pos = cursor + 1;
    }

    /// Extent and flags of the escape sequence starting at `start` (a `\`).
    /// Recognized: `\n \r \t \" \\ \/ \b \f`, `\xNN`, `\uXXXX`. Anything else
    /// keeps its two bytes but is flagged invalid.
    fn escape_extent(&self, start: usize) -> (usize, TokenFlags) {
        let Some(second) = self.peek(start + 1) else {
            return (start + 1, TokenFlags::HAS_ERROR);
        };
        match second {
            b'n' | b'r' | b't' | b'"' | b'\\' | b'/' | b'b' | b'f' => {
                (start + 2, TokenFlags::EMPTY)
            }
            b'x' => {
                if self.hex_run(start + 2, 2) {
                    (start + 4, TokenFlags::EMPTY)
                } else {
                    (self.escape_stop(start, 2), TokenFlags::HAS_ERROR)
                }
            }
            b'u' => {
                if self.hex_run(start + 2, 4) {
                    (start + 6, TokenFlags::EMPTY)
                } else {
                    (self.escape_stop(start, 4), TokenFlags::HAS_ERROR)
                }
            }
            _ => (start + 2, TokenFlags::HAS_ERROR),
        }
    }

    fn hex_run(&self, start: usize, count: usize) -> bool {
        (0..count).all(|offset| self.peek(start + offset).is_some_and(is_hex))
    }

    /// A flagged short escape still consumes the `\x`/`\u` prefix plus however
    /// many hex digits were there, so following bytes lex independently. The
    /// result is always at least two bytes past the backslash.
    fn escape_stop(&self, backslash: usize, wanted: usize) -> usize {
        let from = backslash + 2;
        let mut end = from;
        let mut seen = 0;
        while seen < wanted && self.peek(end).is_some_and(is_hex) {
            end += 1;
            seen += 1;
        }
        end
    }

    /// A heredoc: the `<<TAG` / `<<-TAG` marker, then the body up to a line
    /// holding the tag — at column zero for `<<TAG`, after blanks for `<<-TAG`.
    fn lex_heredoc(&mut self) {
        let start = self.pos;
        let mut cursor = start + 2;
        let indented = self.peek(cursor) == Some(b'-');
        if indented {
            cursor += 1;
        }
        let tag_start = cursor;
        while cursor < self.bytes.len() && is_ident_continue(self.bytes[cursor]) {
            cursor += 1;
        }
        if tag_start == cursor {
            // `<<` with no tag: keep the bytes, flag the fault.
            self.push(SyntaxKind::Error, start, cursor, TokenFlags::HAS_ERROR);
            self.pos = cursor;
            return;
        }
        let tag = &self.bytes[tag_start..cursor];
        self.push(SyntaxKind::HeredocOpen, start, cursor, TokenFlags::EMPTY);
        self.pos = cursor;
        // The rest of the marker line — blanks and its newline — keeps its
        // own tokens; the body starts on the next line.
        while self.pos < self.bytes.len() && is_space(self.bytes[self.pos]) {
            let blank_start = self.pos;
            while self.pos < self.bytes.len() && is_space(self.bytes[self.pos]) {
                self.pos += 1;
            }
            self.push(
                SyntaxKind::Whitespace,
                blank_start,
                self.pos,
                TokenFlags::EMPTY,
            );
        }
        let mut body_start = self.pos;
        if self.pos < self.bytes.len() && is_newline_start(self.bytes[self.pos]) {
            let end = self.newline_end(self.pos);
            self.push(SyntaxKind::Newline, self.pos, end, TokenFlags::EMPTY);
            self.pos = end;
            body_start = end;
        }
        match self.find_heredoc_end(body_start, tag, indented) {
            Some((body_end, close_start)) => {
                self.lex_heredoc_body(body_start, body_end);
                if close_start > body_end {
                    self.push(
                        SyntaxKind::Whitespace,
                        body_end,
                        close_start,
                        TokenFlags::EMPTY,
                    );
                }
                self.push(
                    SyntaxKind::HeredocClose,
                    close_start,
                    close_start + tag.len(),
                    TokenFlags::EMPTY,
                );
                self.pos = close_start + tag.len();
            }
            None => {
                self.lex_heredoc_body(body_start, self.bytes.len());
                self.mark_last_error();
                self.pos = self.bytes.len();
            }
        }
    }

    /// `(body_end, close_start)` for the first line at or after `body_start`
    /// whose content is exactly `tag`; `None` when the heredoc never
    /// terminates. The tag must end its own line.
    ///
    /// `indented` is the `<<-` form: only it may put blanks before the
    /// terminator. For `<<TAG` the tag has to start at column zero, otherwise an
    /// indented line that merely looks like the tag would silently close a
    /// heredoc that HCL still considers open. Trailing blanks are tolerated in
    /// both forms.
    fn find_heredoc_end(
        &self,
        body_start: usize,
        tag: &[u8],
        indented: bool,
    ) -> Option<(usize, usize)> {
        let mut line_start = body_start;
        loop {
            let mut line_end = line_start;
            while line_end < self.bytes.len() && !is_newline_start(self.bytes[line_end]) {
                line_end += 1;
            }
            let mut content_start = line_start;
            if indented {
                while content_start < line_end && is_space(self.bytes[content_start]) {
                    content_start += 1;
                }
            }
            let mut content_end = line_end;
            while content_end > content_start
                && (is_space(self.bytes[content_end - 1]) || self.bytes[content_end - 1] == b'\r')
            {
                content_end -= 1;
            }
            if content_end == line_end
                && content_end - content_start == tag.len()
                && &self.bytes[content_start..content_end] == tag
            {
                return Some((line_start, content_start));
            }
            if line_end >= self.bytes.len() {
                return None;
            }
            line_start = self.newline_end(line_end);
        }
    }

    /// Emits the body between `start` and `end`, splitting `${...}` template
    /// sequences out of the raw runs.
    fn lex_heredoc_body(&mut self, start: usize, end: usize) {
        let mut run_start = start;
        let mut cursor = start;
        while cursor < end {
            if self.bytes[cursor] == b'$' && self.peek(cursor + 1) == Some(b'{') {
                self.push(
                    SyntaxKind::HeredocBody,
                    run_start,
                    cursor,
                    TokenFlags::EMPTY,
                );
                match interpolation_end(self.bytes, cursor + 2) {
                    Some(interp_end) if interp_end <= end => {
                        self.push(
                            SyntaxKind::Interpolation,
                            cursor,
                            interp_end,
                            TokenFlags::EMPTY,
                        );
                        cursor = interp_end;
                    }
                    _ => {
                        // No close within the body: keep the remainder as a
                        // flagged interpolation.
                        self.push(
                            SyntaxKind::Interpolation,
                            cursor,
                            end,
                            TokenFlags::HAS_ERROR,
                        );
                        cursor = end;
                    }
                }
                run_start = cursor;
                continue;
            }
            cursor += 1;
        }
        self.push(SyntaxKind::HeredocBody, run_start, end, TokenFlags::EMPTY);
    }

    fn lex_symbol(&mut self) {
        let start = self.pos;
        let byte = self.bytes[start];
        let next = self.peek(start + 1);
        if matches!(
            (byte, next),
            (b'=', Some(b'=')) | (b'!', Some(b'=')) | (b'&', Some(b'&')) | (b'|', Some(b'|'))
        ) {
            self.push(SyntaxKind::Operator, start, start + 2, TokenFlags::EMPTY);
            self.pos = start + 2;
            return;
        }
        if matches!(
            byte,
            b'=' | b'!' | b'+' | b'-' | b'*' | b'/' | b'%' | b'?' | b':'
        ) {
            self.push(SyntaxKind::Operator, start, start + 1, TokenFlags::EMPTY);
            self.pos = start + 1;
            return;
        }
        if matches!(byte, b'{' | b'}' | b'[' | b']' | b'(' | b')' | b',' | b'.') {
            self.push(SyntaxKind::Punctuation, start, start + 1, TokenFlags::EMPTY);
            self.pos = start + 1;
            return;
        }
        // Unknown byte: keep exactly one whole UTF-8 character, flagged.
        let end = (start + char_width(byte)).min(self.bytes.len());
        self.push(SyntaxKind::Error, start, end, TokenFlags::HAS_ERROR);
        self.pos = end.max(start + 1);
    }

    // ─── token output ────────────────────────────────────────────────────────

    /// Emits a token, skipping zero-width spans so the stream never carries
    /// an empty token.
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

    fn mark_last_error(&mut self) {
        if let Some(last) = self.tokens.last_mut() {
            last.flags = last.flags.with(TokenFlags::HAS_ERROR);
        }
    }
}

/// Byte width of the UTF-8 sequence led by `byte`; continuation-led bytes get
/// width 1 so spans always land on character boundaries the encoder produced.
fn char_width(byte: u8) -> usize {
    match byte {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

/// Position just past the `}` closing the interpolation whose content starts
/// at `start`, tracking nested braces, quoted strings and inner `${...}`.
/// `None` means the input ends first.
fn interpolation_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
                i += 1;
            }
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if i >= bytes.len() {
                    return None;
                }
                i += 1;
            }
            b'$' if bytes.get(i + 1) == Some(&b'{') => {
                let inner = interpolation_end(bytes, i + 2)?;
                i = inner;
            }
            _ => i += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<SyntaxKind> {
        lex(source)
            .tokens()
            .iter()
            .map(|token| token.kind)
            .collect()
    }

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

    fn kinds_and_text(source: &str) -> Vec<(SyntaxKind, &str)> {
        assert_lossless(source);
        lex(source)
            .tokens()
            .iter()
            .map(|token| (token.kind, token.text(source).unwrap()))
            .collect()
    }

    #[test]
    fn attribute_assignment_lexes_exactly() {
        assert_eq!(
            kinds_and_text("port = 8080\n"),
            vec![
                (SyntaxKind::Identifier, "port"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Operator, "="),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Number, "8080"),
                (SyntaxKind::Newline, "\n"),
            ]
        );
    }

    #[test]
    fn identifiers_allow_dash_and_underscore() {
        assert_eq!(
            kinds_and_text("trace-id_2 = 1"),
            vec![
                (SyntaxKind::Identifier, "trace-id_2"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Operator, "="),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Number, "1"),
            ]
        );
    }

    #[test]
    fn comments_carry_two_syntaxes() {
        assert_eq!(
            kinds("# one\n// two\r\n"),
            vec![
                SyntaxKind::LineComment,
                SyntaxKind::Newline,
                SyntaxKind::LineComment,
                SyntaxKind::Newline,
            ]
        );
        assert_eq!(kinds("/* block */"), vec![SyntaxKind::BlockComment]);
        let unterminated = assert_lossless("/* never closed");
        assert!(unterminated.has_errors());
    }

    #[test]
    fn literals_have_their_own_kinds() {
        let observed = kinds_and_text("a = true\nb = false\nc = null");
        assert_eq!(observed[4], (SyntaxKind::Boolean, "true"));
        assert_eq!(observed[10], (SyntaxKind::Boolean, "false"));
        assert_eq!(observed[16], (SyntaxKind::Null, "null"));
    }

    #[test]
    fn numbers_cover_int_float_and_exponent() {
        assert_eq!(
            kinds_and_text("1 0.25 1.5e-3 42."),
            vec![
                (SyntaxKind::Number, "1"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Number, "0.25"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Number, "1.5e-3"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Number, "42"),
                (SyntaxKind::Punctuation, "."),
            ]
        );
    }

    #[test]
    fn operators_have_stable_shapes() {
        assert_eq!(
            kinds("= == != ! && || + - * / % ? :"),
            vec![SyntaxKind::Operator; 13]
                .into_iter()
                .flat_map(|kind| [kind, SyntaxKind::Whitespace])
                .take(25)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn strings_carve_escapes_and_interpolations() {
        assert_eq!(
            kinds_and_text("name = \"hi ${who}!\\n\""),
            vec![
                (SyntaxKind::Identifier, "name"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Operator, "="),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::String, "\"hi "),
                (SyntaxKind::Interpolation, "${who}"),
                (SyntaxKind::String, "!"),
                (SyntaxKind::Escape, "\\n"),
                (SyntaxKind::String, "\""),
            ]
        );
    }

    #[test]
    fn unicode_and_hex_escapes_are_one_token() {
        let observed = kinds_and_text("a = \"\\u00e9\\x41\"");
        assert_eq!(
            observed,
            vec![
                (SyntaxKind::Identifier, "a"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Operator, "="),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::String, "\""),
                (SyntaxKind::Escape, "\\u00e9"),
                (SyntaxKind::Escape, "\\x41"),
                (SyntaxKind::String, "\""),
            ]
        );
    }

    #[test]
    fn invalid_escapes_are_flagged_not_dropped() {
        let lexed = assert_lossless(r#"a = "\q""#);
        assert!(lexed.has_errors());
        let escape = *lexed
            .tokens()
            .iter()
            .find(|token| token.kind == SyntaxKind::Escape)
            .expect("escape token");
        assert!(escape.has_error());
        assert_eq!(escape.text(lexed.source()), Some(r"\q"));
    }

    #[test]
    fn heredocs_split_into_open_body_and_close() {
        let source = "b = <<EOT\nhello\nEOT\n";
        let lexed = assert_lossless(source);
        assert!(!lexed.has_errors());
        let observed: Vec<(SyntaxKind, &str)> = lexed
            .tokens()
            .iter()
            .map(|token| (token.kind, token.text(source).unwrap()))
            .collect();
        assert_eq!(
            observed,
            vec![
                (SyntaxKind::Identifier, "b"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Operator, "="),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::HeredocOpen, "<<EOT"),
                (SyntaxKind::Newline, "\n"),
                (SyntaxKind::HeredocBody, "hello\n"),
                (SyntaxKind::HeredocClose, "EOT"),
                (SyntaxKind::Newline, "\n"),
            ]
        );
    }

    #[test]
    fn indented_terminator_needs_the_dash_form() {
        let source = "b = <<EOT\n  hi\n  EOT\n";
        let lexed = assert_lossless(source);
        assert!(
            lexed.has_errors(),
            "an indented terminator closed a <<EOT heredoc"
        );
        assert!(
            !lexed
                .tokens()
                .iter()
                .any(|t| t.kind == SyntaxKind::HeredocClose)
        );

        // The same document written with `<<-` is valid and does close.
        let dashed = assert_lossless("b = <<-EOT\n  hi\n  EOT\n");
        assert!(!dashed.has_errors());
        assert!(
            dashed
                .tokens()
                .iter()
                .any(|t| t.kind == SyntaxKind::HeredocClose)
        );
    }

    #[test]
    fn indented_heredoc_and_body_interpolation_split() {
        let source = "b = <<-EOT\n  hi ${x}\n  EOT\n";
        let observed = kinds_and_text(source);
        assert_eq!(observed[4], (SyntaxKind::HeredocOpen, "<<-EOT"));
        assert_eq!(observed[6], (SyntaxKind::HeredocBody, "  hi "));
        assert_eq!(observed[7], (SyntaxKind::Interpolation, "${x}"));
        assert_eq!(observed[8], (SyntaxKind::HeredocBody, "\n"));
        assert_eq!(observed[9], (SyntaxKind::Whitespace, "  "));
        assert_eq!(observed[10], (SyntaxKind::HeredocClose, "EOT"));
        assert!(!assert_lossless(source).has_errors());
    }

    #[test]
    fn unterminated_heredoc_keeps_every_byte_flagged() {
        let lexed = assert_lossless("b = <<EOT\nnever ends\n");
        assert!(lexed.has_errors());
        assert!(
            lexed
                .tokens()
                .iter()
                .any(|token| token.kind == SyntaxKind::HeredocBody && token.has_error())
        );
    }

    #[test]
    fn unterminated_string_interpolation_and_brace_survive() {
        assert!(assert_lossless("a = \"oops").has_errors());
        assert!(assert_lossless(r#"a = "${x"#).has_errors());
        assert!(assert_lossless("a = ${").has_errors());
        let open_brace = assert_lossless("block {");
        assert_eq!(
            open_brace
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::Punctuation)
                .count(),
            1
        );
    }

    #[test]
    fn nested_interpolation_and_strings_close_correctly() {
        let observed = kinds_and_text(r#"a = "${f("${g("{x}")}")}""#);
        assert_eq!(observed[4].0, SyntaxKind::String);
        assert_eq!(observed[5].0, SyntaxKind::Interpolation);
        assert_eq!(observed[6].0, SyntaxKind::String);
        assert_eq!(observed[5].1, "${f(\"${g(\"{x}\")}\")}");
    }

    #[test]
    fn stray_bytes_are_error_tokens() {
        let lexed = assert_lossless("a = @ $ b");
        assert!(lexed.has_errors());
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::Error)
                .count(),
            2
        );
    }

    #[test]
    fn bom_crlf_and_non_ascii_are_carried() {
        let source = "\u{FEFF}a = 1\r\né = 2\r\n";
        let lexed = assert_lossless(source);
        assert_eq!(lexed.tokens()[0].kind, SyntaxKind::Bom);
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::Newline)
                .count(),
            2
        );
        // A non-ASCII byte outside a string is an error token, but a whole
        // character's worth of bytes.
        let eacute = lexed
            .tokens()
            .iter()
            .find(|token| token.kind == SyntaxKind::Error)
            .copied()
            .expect("error token");
        assert_eq!(eacute.text(source), Some("é"));
    }

    #[test]
    fn corpus_is_lossless() {
        let corpus: &[&str] = &[
            "",
            "   ",
            "\n\n",
            "\u{FEFF}",
            "a = 1",
            "# c\n// c\n/* c */\n",
            r#"a = "x""#,
            r#"a = "x"#,
            r#"a = "\"""#,
            r#"a = "$""#,
            r#"a = "${}""#,
            "b = <<EOT\ntext\nEOT",
            "b = <<-EOT\n  t\n  EOT\n",
            "b = <<",
            "block { x = 1 }",
            "a = [1, 2,]",
            "a = -(1 + 2) % 3",
            "\u{1F600} = 1",
            "a = 1.2.3",
            "<<EOT\nx\nEOT",
            r#""unterminated with ${interp}" tail"#,
            r#"a = "\u12g4""#,
        ];
        for source in corpus {
            assert_lossless(source);
        }
    }

    #[test]
    fn every_truncation_rebuilds_its_prefix() {
        let sample = concat!(
            "# c\nservice \"api\" { port = 8080\n",
            "note = \"a\\nb ${x}\" banner = <<EOT\nbody ${y}\nEOT\n}\n",
        );
        for cut in 0..=sample.len() {
            if !sample.is_char_boundary(cut) {
                continue;
            }
            assert_lossless(&sample[..cut]);
        }
    }
}
