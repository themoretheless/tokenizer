//! A lossless logfmt lexer.
//!
//! Concatenating token text reconstructs the source byte-for-byte, including
//! for malformed input: an unterminated quoted value, a stray `=` and text
//! welded onto a closing quote all keep their own span — flagged
//! [`LexToken::has_error`] — and are never dropped or replaced by a
//! synthesized token. Nothing here borrows a programming-language vocabulary:
//! a key is a `key`, an `=` is a `separator`, a space inside a record is
//! `whitespace`, and a newline is a `record-break`.
//!
//! The grammar is the whitespace-separated `key=value` stream logfmt
//! documents. A *record* is one line; an *item* inside it is either a
//! `key=value` pair or a stand-alone key (a presence flag); `=` splits at its
//! first occurrence, so `a=b=c` reads as key `a` with bare value `b=c`. A
//! value is either bare (no spaces, no breaks) or double-quoted, and only a
//! quoted value may hold spaces or raw newlines — which is what lets one
//! record continue past its own line. Escapes are exactly `\"` and `\\`; any
//! other backslash is ordinary value text, because a logfmt writer is free to
//! put `\n` in a quoted value as two bytes.
//!
//! The lexer is format-only: `ts`, `level` and `err` are keys like every other
//! key, and no key name changes how a token is classified.

use themoretheless_tokenizer_core::{LosslessViolation, Span, verify_lossless_spans};

/// Exact lexical categories emitted by the logfmt lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxKind {
    /// The UTF-8 encoding of U+FEFF at document start.
    Bom,
    /// The bytes before a `=` that introduces a value.
    Key,
    /// The `=` byte itself, or a stray `=` with no key in front of it.
    Separator,
    /// A value written without quotes: a maximal run with no space or break.
    BareValue,
    /// Literal bytes of a double-quoted value, the surrounding quotes included.
    QuotedValue,
    /// A two-byte `\"` or `\\` sequence inside a quoted value.
    EscapedChar,
    /// A bare token with no `=` after it: a key asserted by presence alone.
    FlagKey,
    /// A run of spaces and tabs inside one record.
    Whitespace,
    /// The `\r\n`, `\n` or `\r` ending a record.
    RecordBreak,
    /// A span the lexer flagged but kept: text welded onto a closing quote.
    Error,
}

impl SyntaxKind {
    /// Trivia separates items and records without carrying log content.
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(self, Self::Bom | Self::Whitespace | Self::RecordBreak)
    }

    /// Kinds holding value bytes, quoted or bare.
    #[must_use]
    pub const fn is_value(self) -> bool {
        matches!(
            self,
            Self::BareValue | Self::QuotedValue | Self::EscapedChar
        )
    }

    /// Kinds holding key bytes, paired or flag-only.
    #[must_use]
    pub const fn is_key(self) -> bool {
        matches!(self, Self::Key | Self::FlagKey)
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

/// A lexical token. Spans are non-empty UTF-8 byte ranges; a value with no
/// bytes (`msg=`) produces no token at all.
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

/// Lexes a logfmt document.
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

/// Item separator. Non-ASCII spaces are deliberately *not* separators: they
/// are legal value bytes, so `msg=\u{A0}x` stays one bare value.
const fn is_space(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

/// Any byte that can start a record break.
const fn is_break(byte: u8) -> bool {
    byte == b'\n' || byte == b'\r'
}

/// Whether a byte ends the run currently being read. Every stop byte is ASCII,
/// so a UTF-8 continuation byte can never split a token mid-character.
const fn ends_run(byte: u8) -> bool {
    is_space(byte) || is_break(byte)
}

impl Lexer<'_> {
    fn run(&mut self) {
        self.lex_bom();
        while self.pos < self.bytes.len() {
            let byte = self.bytes[self.pos];
            if is_space(byte) {
                self.lex_whitespace();
            } else if is_break(byte) {
                let end = self.break_end(self.pos);
                self.push(SyntaxKind::RecordBreak, self.pos, end, TokenFlags::EMPTY);
                self.pos = end;
            } else if byte == b'=' {
                // A separator with no key in front of it: kept and flagged.
                self.push(
                    SyntaxKind::Separator,
                    self.pos,
                    self.pos + 1,
                    TokenFlags::HAS_ERROR,
                );
                self.pos += 1;
            } else {
                self.lex_item();
            }
        }
    }

    fn lex_bom(&mut self) {
        if self.bytes.starts_with(&BOM) {
            self.push(SyntaxKind::Bom, 0, BOM.len(), TokenFlags::EMPTY);
            self.pos = BOM.len();
        }
    }

    /// End of the record break starting at `start`: `\r\n` is one break.
    fn break_end(&self, start: usize) -> usize {
        let mut end = start + 1;
        if self.bytes[start] == b'\r' && self.bytes.get(end) == Some(&b'\n') {
            end += 1;
        }
        end
    }

    /// A maximal run of spaces and tabs between two items.
    fn lex_whitespace(&mut self) {
        let start = self.pos;
        let mut end = start;
        while end < self.bytes.len() && is_space(self.bytes[end]) {
            end += 1;
        }
        self.push(SyntaxKind::Whitespace, start, end, TokenFlags::EMPTY);
        self.pos = end;
    }

    /// One item: the key run up to a space, a break or the first `=`, then
    /// either a separator plus a value or nothing at all (a flag key).
    fn lex_item(&mut self) {
        let start = self.pos;
        let mut end = start;
        while end < self.bytes.len() && !ends_run(self.bytes[end]) && self.bytes[end] != b'=' {
            end += 1;
        }
        let paired = self.bytes.get(end) == Some(&b'=');
        let kind = if paired {
            SyntaxKind::Key
        } else {
            SyntaxKind::FlagKey
        };
        self.push(kind, start, end, TokenFlags::EMPTY);
        self.pos = end;
        if paired {
            self.push(
                SyntaxKind::Separator,
                self.pos,
                self.pos + 1,
                TokenFlags::EMPTY,
            );
            self.pos += 1;
            self.lex_value();
        }
    }

    /// The value after a separator. An empty value contributes no token — a
    /// zero-width span is forbidden — and the structure layer records the pair
    /// as `key=` with no value bytes.
    fn lex_value(&mut self) {
        match self.bytes.get(self.pos) {
            Some(&b'"') => self.lex_quoted_value(),
            Some(&byte) if !ends_run(byte) => self.lex_bare_value(),
            _ => {}
        }
    }

    fn lex_bare_value(&mut self) {
        let start = self.pos;
        let mut end = start;
        while end < self.bytes.len() && !ends_run(self.bytes[end]) {
            end += 1;
        }
        self.push(SyntaxKind::BareValue, start, end, TokenFlags::EMPTY);
        self.pos = end;
    }

    /// A double-quoted value: literal runs interleaved with `\"` and `\\`
    /// escapes, the surrounding quotes carried by the runs themselves so every
    /// byte stays covered. An unterminated value flags its opening run; text
    /// welded onto the closing quote is kept as an `error` run.
    fn lex_quoted_value(&mut self) {
        let open = self.pos;
        let first = self.tokens.len();
        let mut run_start = open;
        let mut cursor = open + 1;
        let mut closed = false;
        while cursor < self.bytes.len() {
            let byte = self.bytes[cursor];
            if byte == b'"' {
                cursor += 1;
                self.push(
                    SyntaxKind::QuotedValue,
                    run_start,
                    cursor,
                    TokenFlags::EMPTY,
                );
                closed = true;
                break;
            }
            if byte == b'\\' && matches!(self.bytes.get(cursor + 1), Some(&b'"') | Some(&b'\\')) {
                self.push(
                    SyntaxKind::QuotedValue,
                    run_start,
                    cursor,
                    TokenFlags::EMPTY,
                );
                cursor += 2;
                self.push(
                    SyntaxKind::EscapedChar,
                    cursor - 2,
                    cursor,
                    TokenFlags::EMPTY,
                );
                run_start = cursor;
                continue;
            }
            cursor += 1;
        }
        if !closed {
            self.push(
                SyntaxKind::QuotedValue,
                run_start,
                self.bytes.len(),
                TokenFlags::EMPTY,
            );
            self.mark_error(first);
            self.pos = self.bytes.len();
            return;
        }
        // The value is complete only once whitespace or a break follows: any
        // other byte is stray text that belongs to no item.
        let mut end = cursor;
        while end < self.bytes.len() && !ends_run(self.bytes[end]) {
            end += 1;
        }
        if end > cursor {
            self.push(SyntaxKind::Error, cursor, end, TokenFlags::HAS_ERROR);
        }
        self.pos = end;
    }

    // ─── token output ────────────────────────────────────────────────────────

    /// Emits a token and returns its index. Zero-width spans are skipped so
    /// the stream never carries an empty token: `msg=` holds no value bytes and
    /// so contributes no value token.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Documents the writer output should never be flagged: every one of these
    /// must lex clean *and* losslessly.
    const CLEAN_CORPUS: &[&str] = &[
        "msg=hello\n",
        "msg=hello",
        "level=info msg=\"starting server\" addr=0.0.0.0:8080\n",
        "msg=\"say \\\"hi\\\"\"\n",
        "path=\"C:\\\\tmp\\\\file\"\n",
        "msg=\"multi\nline\" tail=1\n",
        "trace-id=deadbeef -v --dry-run\n",
        "msg=\n",
        "a=b=c\n",
        "msg=値 emoji=😀\n",
        "\u{FEFF}level=warn msg=\"with bom\"\n",
        "dur=12ms err=null n=007\n",
    ];

    /// Malformed or merely unusual input: still byte-for-byte recoverable.
    const BROKEN_CORPUS: &[&str] = &[
        "msg=\"oops\n",
        "msg=\"oops",
        "msg=\"oops\"x\n",
        "=oops\n",
        "msg = hi\n",
        "msg=\"a\\",
        "\"a b\"=c\n",
        "msg=he\"llo",
        "msg=\"\\",
        "",
        "\n",
        "\n\n",
        " ",
        "  \t  \n",
        "\r\r\n\r\n",
        "trailing=break\r\n",
        "名前,点数=1\n",
        "msg=\"未完",
        "msg=😀",
        "a=\"x\"\"y\"\n",
        "a=\"\"b\n",
    ];

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

    /// `(kind, text)` for every token, so a test states the whole reading.
    fn kinds_and_text(source: &str) -> Vec<(SyntaxKind, &str)> {
        assert_lossless(source);
        lex(source)
            .tokens()
            .iter()
            .map(|token| (token.kind, token.text(source).unwrap()))
            .collect()
    }

    #[test]
    fn corpus_is_lossless() {
        for source in CLEAN_CORPUS.iter().chain(BROKEN_CORPUS) {
            assert_lossless(source);
        }
    }

    #[test]
    fn simple_pair_kinds_are_exact() {
        assert_eq!(
            kinds("msg=hello\n"),
            vec![
                SyntaxKind::Key,
                SyntaxKind::Separator,
                SyntaxKind::BareValue,
                SyntaxKind::RecordBreak,
            ]
        );
        assert_eq!(
            kinds("msg=hello level=info\n"),
            vec![
                SyntaxKind::Key,
                SyntaxKind::Separator,
                SyntaxKind::BareValue,
                SyntaxKind::Whitespace,
                SyntaxKind::Key,
                SyntaxKind::Separator,
                SyntaxKind::BareValue,
                SyntaxKind::RecordBreak,
            ]
        );
    }

    #[test]
    fn bare_value_run_is_one_token() {
        assert_eq!(
            kinds_and_text("msg=a:b:c/d\n"),
            vec![
                (SyntaxKind::Key, "msg"),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::BareValue, "a:b:c/d"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
    }

    #[test]
    fn equals_splits_the_key_at_its_first_occurrence() {
        assert_eq!(
            kinds_and_text("expr=a=b=c\n"),
            vec![
                (SyntaxKind::Key, "expr"),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::BareValue, "a=b=c"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
    }

    #[test]
    fn empty_value_contributes_no_token() {
        let lexed = assert_lossless("msg=\n");
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .map(|token| token.kind)
                .collect::<Vec<_>>(),
            vec![
                SyntaxKind::Key,
                SyntaxKind::Separator,
                SyntaxKind::RecordBreak,
            ]
        );
        assert!(!lexed.has_errors(), "an empty value is valid logfmt");
        // At end of input the pair is still just key plus separator.
        assert_eq!(kinds("msg="), vec![SyntaxKind::Key, SyntaxKind::Separator]);
        // An empty value between two pairs keeps both of them.
        assert_eq!(
            kinds_and_text("a= b=c\n"),
            vec![
                (SyntaxKind::Key, "a"),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Key, "b"),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::BareValue, "c"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
    }

    #[test]
    fn quoted_value_keeps_its_own_quotes() {
        assert_eq!(
            kinds_and_text("msg=\"hello world\"\n"),
            vec![
                (SyntaxKind::Key, "msg"),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::QuotedValue, "\"hello world\""),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
        let empty = kinds_and_text("msg=\"\"\n");
        assert_eq!(
            empty[2],
            (SyntaxKind::QuotedValue, "\"\""),
            "a quoted empty value has bytes and so is a token"
        );
    }

    #[test]
    fn escape_sequences_are_carved_out_as_escaped_char() {
        assert_eq!(
            kinds_and_text("msg=\"say \\\"hi\\\"\"\n"),
            vec![
                (SyntaxKind::Key, "msg"),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::QuotedValue, "\"say "),
                (SyntaxKind::EscapedChar, "\\\""),
                (SyntaxKind::QuotedValue, "hi"),
                (SyntaxKind::EscapedChar, "\\\""),
                (SyntaxKind::QuotedValue, "\""),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
    }

    #[test]
    fn backslash_before_a_value_byte_stays_literal() {
        // Only \" and \\ escape: \n is two ordinary bytes of quoted text.
        assert_eq!(
            kinds_and_text(r#"msg="line\nnext""#),
            vec![
                (SyntaxKind::Key, "msg"),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::QuotedValue, "\"line\\nnext\""),
            ]
        );
        let doubled = kinds_and_text(r#"path="C:\\tmp""#);
        assert_eq!(
            doubled[2..4],
            [
                (SyntaxKind::QuotedValue, "\"C:"),
                (SyntaxKind::EscapedChar, "\\\\"),
            ]
        );
        assert_eq!(
            doubled.last().copied(),
            Some((SyntaxKind::QuotedValue, "tmp\""))
        );
    }

    #[test]
    fn raw_newlines_inside_a_quoted_value_belong_to_it() {
        let lexed = assert_lossless("msg=\"two\nlines\" tail=1\n");
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::RecordBreak)
                .count(),
            1,
            "the break inside the quotes must not split the record"
        );
        let content = lexed
            .tokens()
            .iter()
            .find(|token| token.text(lexed.source()) == Some("\"two\nlines\""))
            .copied()
            .expect("multi-line quoted value");
        assert_eq!(content.kind, SyntaxKind::QuotedValue);
        assert!(!content.has_error());
    }

    #[test]
    fn flag_keys_are_their_own_kind() {
        assert_eq!(
            kinds_and_text("-v --dry-run on\n"),
            vec![
                (SyntaxKind::FlagKey, "-v"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::FlagKey, "--dry-run"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::FlagKey, "on"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
        // A dash-prefixed key can also be paired like any other.
        assert_eq!(
            kinds("--flag=1\n"),
            vec![
                SyntaxKind::Key,
                SyntaxKind::Separator,
                SyntaxKind::BareValue,
                SyntaxKind::RecordBreak,
            ]
        );
        let lone_dash = kinds_and_text("-\n");
        assert_eq!(lone_dash[0], (SyntaxKind::FlagKey, "-"));
    }

    #[test]
    fn leading_whitespace_trims_to_the_first_item() {
        assert_eq!(
            kinds("   msg=x\n"),
            vec![
                SyntaxKind::Whitespace,
                SyntaxKind::Key,
                SyntaxKind::Separator,
                SyntaxKind::BareValue,
                SyntaxKind::RecordBreak,
            ]
        );
        assert_eq!(
            kinds("\t \ta=b\t\tc=d"),
            vec![
                SyntaxKind::Whitespace,
                SyntaxKind::Key,
                SyntaxKind::Separator,
                SyntaxKind::BareValue,
                SyntaxKind::Whitespace,
                SyntaxKind::Key,
                SyntaxKind::Separator,
                SyntaxKind::BareValue,
            ]
        );
    }

    #[test]
    fn significant_tokens_drop_only_trivia() {
        let lexed = lex("  msg=x  \n");
        assert_eq!(
            lexed
                .significant_tokens()
                .map(|token| token.kind)
                .collect::<Vec<_>>(),
            vec![
                SyntaxKind::Key,
                SyntaxKind::Separator,
                SyntaxKind::BareValue,
            ]
        );
        assert!(SyntaxKind::Whitespace.is_trivia());
        assert!(SyntaxKind::BareValue.is_value());
        assert!(SyntaxKind::FlagKey.is_key());
        assert!(!SyntaxKind::Separator.is_trivia());
    }

    #[test]
    fn every_break_shape_is_one_record_break_token() {
        for line_ending in ["\n", "\r\n", "\r"] {
            let source = format!("msg=x{line_ending}y=1");
            let lexed = assert_lossless(&source);
            assert_eq!(
                lexed
                    .tokens()
                    .iter()
                    .filter(|token| token.kind == SyntaxKind::RecordBreak)
                    .count(),
                1,
                "{source:?}"
            );
        }
        assert_eq!(kinds("\r\n"), vec![SyntaxKind::RecordBreak]);
    }

    #[test]
    fn unterminated_quote_is_flagged_not_dropped() {
        let lexed = assert_lossless("msg=\"oops\nmore\n");
        assert!(lexed.has_errors());
        let flagged: Vec<LexToken> = lexed
            .tokens()
            .iter()
            .copied()
            .filter(|token| token.has_error())
            .collect();
        assert_eq!(flagged.len(), 1, "exactly one span carries the fault");
        assert_eq!(flagged[0].kind, SyntaxKind::QuotedValue);
        assert_eq!(flagged[0].text(lexed.source()), Some("\"oops\nmore\n"));
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::RecordBreak)
                .count(),
            0,
            "the swallowed breaks are value bytes now"
        );
        assert_eq!(kinds("msg=\"oops").len(), 3);
    }

    #[test]
    fn stray_equals_is_a_flagged_separator() {
        let lexed = assert_lossless("=oops msg=x\n");
        let first = lexed.tokens()[0];
        assert_eq!(first.kind, SyntaxKind::Separator);
        assert!(first.has_error());
        assert_eq!(first.span, Span::new(0, 1));
        assert_eq!(kinds("a==b\n").len(), 4);
    }

    #[test]
    fn text_glued_to_a_closing_quote_is_kept_as_error() {
        let lexed = assert_lossless("msg=\"hi\"there x=1\n");
        let errors: Vec<LexToken> = lexed
            .tokens()
            .iter()
            .copied()
            .filter(|token| token.kind == SyntaxKind::Error)
            .collect();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].text(lexed.source()), Some("there"));
        assert!(errors[0].has_error());
        assert!(!lexed.tokens()[2].has_error(), "the value itself is fine");
    }

    #[test]
    fn bom_is_a_single_leading_token() {
        let lexed = assert_lossless("\u{FEFF}msg=x\n");
        let first = lexed.tokens()[0];
        assert_eq!(first.kind, SyntaxKind::Bom);
        assert_eq!(first.span, Span::new(0, 3));
        assert_eq!(lexed.tokens()[1].kind, SyntaxKind::Key);
        assert_eq!(kinds("\u{FEFF}"), vec![SyntaxKind::Bom]);
    }

    #[test]
    fn no_key_name_is_special_cased() {
        // ts/level/err/msg style keys lex identically to any other key: this is
        // a format engine, not a log pipeline.
        for source in [
            "ts=x level=x err=x msg=x",
            "a=x b=x c=x d=x",
            "timestamp=1 error=null log.level=info caller=x",
        ] {
            let observed = kinds(source);
            let letters: Vec<SyntaxKind> = observed
                .iter()
                .copied()
                .filter(|kind| !kind.is_trivia() && *kind != SyntaxKind::Separator)
                .collect();
            assert_eq!(
                letters,
                vec![
                    SyntaxKind::Key,
                    SyntaxKind::BareValue,
                    SyntaxKind::Key,
                    SyntaxKind::BareValue,
                    SyntaxKind::Key,
                    SyntaxKind::BareValue,
                    SyntaxKind::Key,
                    SyntaxKind::BareValue,
                ],
                "{source:?}"
            );
        }
    }

    #[test]
    fn unicode_never_splits_inside_a_character() {
        // A value with a space in it has to be quoted, so the second token is
        // its own item: a multi-byte flag key, not a continuation of `名前`.
        assert_eq!(
            kinds_and_text("msg=名前 🚀\n"),
            vec![
                (SyntaxKind::Key, "msg"),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::BareValue, "名前"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::FlagKey, "🚀"),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
        assert_eq!(
            kinds_and_text("msg=\"名前 🚀\"\n"),
            vec![
                (SyntaxKind::Key, "msg"),
                (SyntaxKind::Separator, "="),
                (SyntaxKind::QuotedValue, "\"名前 🚀\""),
                (SyntaxKind::RecordBreak, "\n"),
            ]
        );
        // A quoted value with unicode *and* escapes still partitions exactly:
        // the runs reassemble the value, quotes and backslashes included.
        let source = "msg=\"🚀 名前 \\\"ok\\\"\"\n";
        let lexed = assert_lossless(source);
        let runs: Vec<&str> = lexed
            .tokens()
            .iter()
            .filter(|token| token.kind.is_value())
            .map(|token| token.text(source).unwrap())
            .collect();
        let value_region = &source["msg=".len()..source.len() - "\n".len()];
        assert_eq!(runs.len(), 5, "{runs:?}: three value runs, two escapes");
        assert_eq!(
            runs.concat(),
            value_region,
            "the value runs rebuild the quoted region byte-for-byte"
        );
        let escapes: Vec<LexToken> = lexed
            .tokens()
            .iter()
            .copied()
            .filter(|token| token.kind == SyntaxKind::EscapedChar)
            .collect();
        assert_eq!(escapes.len(), 2, "each escape is its own token");
        for escape in escapes {
            assert_eq!(escape.text(source), Some("\\\""));
        }
        // A non-ASCII space is value text, never an item separator.
        assert_eq!(
            kinds("msg=a\u{A0}b\n"),
            vec![
                SyntaxKind::Key,
                SyntaxKind::Separator,
                SyntaxKind::BareValue,
                SyntaxKind::RecordBreak,
            ]
        );
    }

    #[test]
    fn empty_and_whitespace_only_input_lex_to_little() {
        let empty = assert_lossless("");
        assert!(empty.tokens().is_empty());
        assert!(empty.is_lossless());
        assert_eq!(assert_lossless("\n").tokens().len(), 1);
        assert_eq!(assert_lossless(" ").tokens().len(), 1);
        assert_eq!(assert_lossless("\n\n").tokens().len(), 2);
    }

    #[test]
    fn every_truncation_rebuilds_its_prefix() {
        let sample =
            "a=1 msg=\"two \"\"words\"\r\nlevel=info -v x=\"un\nclosed\"tail=\"\n=orphan b=c\n";
        for cut in 0..=sample.len() {
            assert_lossless(&sample[..cut]);
        }
    }

    #[test]
    fn every_character_boundary_truncation_rebuilds_its_prefix() {
        let sample = "msg=\"名前 🚀\" key=値 tail=\"off\r\n=orphan 未完=\"x\n";
        for cut in 0..=sample.len() {
            if !sample.is_char_boundary(cut) {
                continue;
            }
            assert_lossless(&sample[..cut]);
        }
    }

    #[test]
    fn long_and_wide_records_stay_lossless() {
        let wide = format!("msg={} end=1\n", "v ".repeat(200));
        assert_lossless(&wide);
        let deep_quotes = format!("msg=\"{}x", "\\\"".repeat(64));
        assert_lossless(&deep_quotes);
        let many_pairs = format!("{}last=1\n", "k=v ".repeat(300));
        assert_lossless(&many_pairs);
    }
}
