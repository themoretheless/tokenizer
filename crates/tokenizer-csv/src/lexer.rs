//! A lossless, delimiter-parameterised CSV/TSV lexer.
//!
//! Concatenating token text reconstructs the source byte-for-byte, including
//! for malformed input: an unterminated quoted field or stray text after a
//! closing quote keeps its own span — flagged [`LexToken::has_error`] — and is
//! never dropped or replaced by a synthesized token. Nothing here borrows a
//! programming-language vocabulary: a comma is a `delimiter`, a newline is a
//! `record-break`, and a space is ordinary field text, so the stream has no
//! `whitespace` kind at all.
//!
//! CSV and TSV share this lexer and differ only through [`Options`]: CSV uses
//! the comma with RFC 4180 quoting (a field starting with `"` may contain
//! delimiters, record breaks, and `""` escapes), while TSV uses the tab with
//! quoting off, where `"` is field text and a field never spans a break.

use themoretheless_tokenizer_core::Span;

/// Which byte separates fields, and whether `"` opens a quoted field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Delimiter {
    /// RFC 4180 CSV: comma-separated, quoting enabled.
    Comma,
    /// Tab-separated values: quoting is not meaningful.
    Tab,
}

impl Delimiter {
    /// The separating byte.
    #[must_use]
    pub const fn byte(self) -> u8 {
        match self {
            Self::Comma => b',',
            Self::Tab => b'\t',
        }
    }

    /// Whether a `"` at a field start opens a quoted field for this delimiter.
    #[must_use]
    pub const fn allows_quoting(self) -> bool {
        matches!(self, Self::Comma)
    }
}

/// Engine options: the field delimiter plus the explicit quoting switch, so
/// a future semicolon dialect can pick any combination of the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Options {
    pub delimiter: Delimiter,
    /// With quoting off (TSV) a `"` is ordinary field text and no field can
    /// contain a delimiter or a record break.
    pub quoting: bool,
}

impl Options {
    /// Comma-separated values with RFC 4180 quoting.
    pub const CSV: Self = Self {
        delimiter: Delimiter::Comma,
        quoting: true,
    };

    /// Tab-separated values; quotes are field text.
    pub const TSV: Self = Self {
        delimiter: Delimiter::Tab,
        quoting: false,
    };
}

impl Default for Options {
    fn default() -> Self {
        Self::CSV
    }
}

/// Exact lexical categories emitted by the CSV/TSV lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxKind {
    /// The UTF-8 encoding of U+FEFF at document start.
    Bom,
    /// Field text belonging to the first (header) record.
    HeaderField,
    /// Field text of any later record.
    Field,
    /// One individual `"` character, opening or closing a quoted field.
    Quote,
    /// A `""` pair inside a quoted field, standing for one literal `"`.
    EscapedQuote,
    /// The field separator byte itself.
    Delimiter,
    /// The `\r\n`, `\n` or `\r` ending a record.
    RecordBreak,
    /// A span the lexer flagged but kept: stray text after a closing quote.
    Error,
}

impl SyntaxKind {
    /// Trivia carries no CSV value structure.
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(self, Self::Bom | Self::RecordBreak)
    }

    /// Kinds holding field text.
    #[must_use]
    pub const fn is_field_text(self) -> bool {
        matches!(self, Self::HeaderField | Self::Field)
    }
}

/// Compact per-token state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TokenFlags(u8);

impl TokenFlags {
    pub const EMPTY: Self = Self(0);
    /// The span is part of, or abandoned by, a malformed construct.
    pub const HAS_ERROR: Self = Self(1);
    /// The field text sits inside a quoted CSV field, so its bytes must not
    /// be read as a bare number or boolean.
    pub const QUOTED: Self = Self(2);

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

/// A lexical token. Spans are non-empty UTF-8 byte ranges; a zero-length
/// field produces no token at all.
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

    /// Whether this field-text token was lexed inside a quoted field.
    #[must_use]
    pub const fn is_quoted(self) -> bool {
        self.flags.contains(TokenFlags::QUOTED)
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

/// Lexes CSV or TSV depending on `options`.
#[must_use]
pub fn lex(source: &str, options: Options) -> Lexed<'_> {
    let mut lexer = Lexer {
        bytes: source.as_bytes(),
        pos: 0,
        tokens: Vec::new(),
        delimiter: options.delimiter.byte(),
        quoting: options.quoting,
        in_header: true,
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
    delimiter: u8,
    quoting: bool,
    in_header: bool,
}

/// UTF-8 encoding of U+FEFF.
const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

impl Lexer<'_> {
    fn run(&mut self) {
        self.lex_bom();
        while self.pos < self.bytes.len() {
            let byte = self.bytes[self.pos];
            if byte == self.delimiter {
                self.push(
                    SyntaxKind::Delimiter,
                    self.pos,
                    self.pos + 1,
                    TokenFlags::EMPTY,
                );
                self.pos += 1;
            } else if byte == b'\n' || byte == b'\r' {
                let end = self.break_end(self.pos);
                self.push(SyntaxKind::RecordBreak, self.pos, end, TokenFlags::EMPTY);
                self.in_header = false;
                self.pos = end;
            } else if byte == b'"' && self.quoting {
                self.lex_quoted_field();
            } else {
                self.lex_plain_field();
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

    /// Field text from `pos` up to the next delimiter, record break or EOF.
    /// An unquoted field may hold `"` bytes; only a field that *starts* with a
    /// quote is quoted, which is exactly what the CSV spec leaves lenient.
    fn lex_plain_field(&mut self) {
        let start = self.pos;
        let mut end = start;
        while end < self.bytes.len()
            && self.bytes[end] != self.delimiter
            && self.bytes[end] != b'\n'
            && self.bytes[end] != b'\r'
        {
            end += 1;
        }
        self.push(self.field_kind(), start, end, TokenFlags::EMPTY);
        self.pos = end;
    }

    /// A quoted field: opener, content runs (which may hold delimiters,
    /// record breaks and `""` escapes), and a closer. Everything malformed is
    /// kept and flagged; the stream never loses bytes.
    fn lex_quoted_field(&mut self) {
        let opener = self
            .push(SyntaxKind::Quote, self.pos, self.pos + 1, TokenFlags::EMPTY)
            .expect("a one-byte quote always produces a token");
        self.pos += 1;
        loop {
            let start = self.pos;
            let mut end = start;
            while end < self.bytes.len() && self.bytes[end] != b'"' {
                end += 1;
            }
            self.push(self.field_kind(), start, end, TokenFlags::QUOTED);
            self.pos = end;
            if self.pos >= self.bytes.len() {
                // EOF reached inside the quoted field: the opener carries the
                // abandonment, exactly like an unclosed code fence does.
                self.mark_error(opener);
                return;
            }
            if self.bytes.get(self.pos + 1) == Some(&b'"') {
                self.push(
                    SyntaxKind::EscapedQuote,
                    self.pos,
                    self.pos + 2,
                    TokenFlags::QUOTED,
                );
                self.pos += 2;
                continue;
            }
            self.push(SyntaxKind::Quote, self.pos, self.pos + 1, TokenFlags::EMPTY);
            self.pos += 1;
            if self.at_field_end() {
                return;
            }
            // Anything between the closing quote and the next delimiter or
            // record break is stray text: kept, flagged, and reported.
            let start = self.pos;
            let mut end = start;
            while end < self.bytes.len() && !self.is_terminator(self.bytes[end]) {
                end += 1;
            }
            self.push(SyntaxKind::Error, start, end, TokenFlags::HAS_ERROR);
            self.pos = end;
            return;
        }
    }

    /// Whether the closing quote of a field ended the field.
    fn at_field_end(&self) -> bool {
        self.pos >= self.bytes.len() || self.is_terminator(self.bytes[self.pos])
    }

    fn is_terminator(&self, byte: u8) -> bool {
        byte == self.delimiter || byte == b'\n' || byte == b'\r'
    }

    /// Text tokens are `header-field` until the first record break outside
    /// any quoted region has been emitted.
    fn field_kind(&self) -> SyntaxKind {
        if self.in_header {
            SyntaxKind::HeaderField
        } else {
            SyntaxKind::Field
        }
    }

    // ─── token output ────────────────────────────────────────────────────────

    /// Emits a token and returns its index. Zero-width spans are skipped so
    /// the stream never carries an empty token: a zero-length field between
    /// two delimiters contributes no bytes and no token.
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

    fn kinds(source: &str, options: Options) -> Vec<SyntaxKind> {
        lex(source, options)
            .tokens()
            .iter()
            .map(|token| token.kind)
            .collect()
    }

    fn assert_lossless(source: &str, options: Options) -> Lexed<'_> {
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
    fn corpus_is_lossless() {
        let csv_corpus = [
            "name,score\nada,42\n",
            "a,b,c\n,,\n",
            "x,\"quoted, comma\"\n",
            "a,\"line one\r\nline two\",b\n",
            "q,\"with \"\"escape\"\" inside\"\n",
            "\"007\",3,true\n",
            "name,score\nada,42,extra\n",
            "a,\"unclosed\n",
            "a,\"unclosed at eof",
            "a,\"x\"y,b\n",
            "\u{FEFF}name,value\nx,1\n",
            "trailing,break\r\n",
            "",
            "\n",
            ",,,\n",
            ",",
            "\"\"",
            "\"\"\"\"\n",
            "plain\"quote\"",
            "\r\r\n\r\n",
            "名前,点数\n真美,9.5\n",
        ];
        for source in csv_corpus {
            assert_lossless(source, Options::CSV);
        }
        let tsv_corpus = [
            "name\tscore\nada\t42\n",
            "quote\"\tstill text\n",
            "comma,is\tordinary\n",
            "\"unclosed\n",
            "\u{FEFF}a\tb\n",
            "",
            "\t\t",
        ];
        for source in tsv_corpus {
            assert_lossless(source, Options::TSV);
        }
    }

    #[test]
    fn simple_row_kinds_are_exact() {
        assert_eq!(
            kinds("name,score\n", Options::CSV),
            vec![
                SyntaxKind::HeaderField,
                SyntaxKind::Delimiter,
                SyntaxKind::HeaderField,
                SyntaxKind::RecordBreak,
            ]
        );
        assert_eq!(
            kinds("name,score\nada,42\n", Options::CSV),
            vec![
                SyntaxKind::HeaderField,
                SyntaxKind::Delimiter,
                SyntaxKind::HeaderField,
                SyntaxKind::RecordBreak,
                SyntaxKind::Field,
                SyntaxKind::Delimiter,
                SyntaxKind::Field,
                SyntaxKind::RecordBreak,
            ]
        );
    }

    #[test]
    fn quoted_field_splits_into_quote_content_quote() {
        let lexed = assert_lossless("a,\"b,c\",d\n", Options::CSV);
        let tokens = lexed.tokens();
        assert_eq!(
            tokens
                .iter()
                .map(|token| (token.kind, token.text(lexed.source())))
                .collect::<Vec<_>>(),
            vec![
                (SyntaxKind::HeaderField, Some("a")),
                (SyntaxKind::Delimiter, Some(",")),
                (SyntaxKind::Quote, Some("\"")),
                (SyntaxKind::HeaderField, Some("b,c")),
                (SyntaxKind::Quote, Some("\"")),
                (SyntaxKind::Delimiter, Some(",")),
                (SyntaxKind::HeaderField, Some("d")),
                (SyntaxKind::RecordBreak, Some("\n")),
            ]
        );
        assert!(tokens[3].is_quoted());
        assert!(!tokens[2].is_quoted());
    }

    #[test]
    fn escaped_quote_pair_is_its_own_token() {
        let lexed = assert_lossless("say,\"he said \"\"hi\"\"\"\n", Options::CSV);
        let quotes: Vec<LexToken> = lexed
            .tokens()
            .iter()
            .copied()
            .filter(|token| {
                token.kind == SyntaxKind::Quote || token.kind == SyntaxKind::EscapedQuote
            })
            .collect();
        assert_eq!(
            quotes
                .iter()
                .map(|token| (token.kind, token.text(lexed.source())))
                .collect::<Vec<_>>(),
            vec![
                (SyntaxKind::Quote, Some("\"")),
                (SyntaxKind::EscapedQuote, Some("\"\"")),
                (SyntaxKind::EscapedQuote, Some("\"\"")),
                (SyntaxKind::Quote, Some("\"")),
            ]
        );
        assert!(quotes[1].is_quoted());
    }

    #[test]
    fn record_breaks_inside_a_quoted_field_stay_field_text() {
        let lexed = assert_lossless("a,\"one\r\ntwo\",b\n", Options::CSV);
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::RecordBreak)
                .count(),
            1,
            "the CRLF inside the quotes must not split the record"
        );
        let content = lexed
            .tokens()
            .iter()
            .find(|token| token.text(lexed.source()) == Some("one\r\ntwo"))
            .copied()
            .expect("multi-line content");
        assert_eq!(content.kind, SyntaxKind::HeaderField);
        assert!(content.is_quoted());
    }

    #[test]
    fn empty_fields_produce_no_tokens() {
        let lexed = assert_lossless("a,,b\n", Options::CSV);
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind.is_field_text())
                .count(),
            2
        );
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::Delimiter)
                .count(),
            2
        );
    }

    #[test]
    fn header_fields_end_with_the_first_record_break() {
        let lexed = assert_lossless("h1,h2\nbody\n", Options::CSV);
        let header = lexed
            .tokens()
            .iter()
            .filter(|token| token.kind == SyntaxKind::HeaderField)
            .count();
        assert_eq!(header, 2);
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::Field)
                .count(),
            1
        );
    }

    #[test]
    fn broken_constructs_are_flagged_not_dropped() {
        let unclosed = assert_lossless("a,\"oops\n", Options::CSV);
        assert!(unclosed.has_errors());
        assert!(
            unclosed
                .tokens()
                .iter()
                .any(|token| token.kind == SyntaxKind::Quote && token.has_error())
        );
        let stray = assert_lossless("a,\"x\"y,b\n", Options::CSV);
        assert!(stray.has_errors());
        assert!(
            stray
                .tokens()
                .iter()
                .any(|token| token.kind == SyntaxKind::Error)
        );
        assert!(!stray.tokens().iter().any(|token| {
            token.has_error() && matches!(token.kind, SyntaxKind::Quote | SyntaxKind::Field)
        }));
    }

    #[test]
    fn bom_is_a_single_leading_token() {
        let lexed = assert_lossless("\u{FEFF}a,b\n", Options::CSV);
        let first = lexed.tokens()[0];
        assert_eq!(first.kind, SyntaxKind::Bom);
        assert_eq!(first.span, Span::new(0, 3));
        assert_eq!(
            lexed.tokens()[1].kind,
            SyntaxKind::HeaderField,
            "the BOM must not turn the header into a body field"
        );
        assert_eq!(kinds("\u{FEFF}", Options::CSV), vec![SyntaxKind::Bom]);
    }

    #[test]
    fn tsv_quotes_and_commas_are_field_text() {
        let lexed = assert_lossless("a\"b,c\"d\te\n", Options::TSV);
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .map(|token| (token.kind, token.text(lexed.source())))
                .collect::<Vec<_>>(),
            vec![
                (SyntaxKind::HeaderField, Some("a\"b,c\"d")),
                (SyntaxKind::Delimiter, Some("\t")),
                (SyntaxKind::HeaderField, Some("e")),
                (SyntaxKind::RecordBreak, Some("\n")),
            ]
        );
        assert!(!lexed.has_errors());
    }

    #[test]
    fn tsv_delimiter_is_the_tab_not_the_comma() {
        assert_eq!(
            kinds("a,b\tc\n", Options::TSV),
            vec![
                SyntaxKind::HeaderField,
                SyntaxKind::Delimiter,
                SyntaxKind::HeaderField,
                SyntaxKind::RecordBreak,
            ]
        );
        assert_eq!(
            kinds("a,b\tc\n", Options::CSV)
                .iter()
                .filter(|kind| **kind == SyntaxKind::Delimiter)
                .count(),
            1
        );
    }

    #[test]
    fn every_break_shape_is_one_record_break_token() {
        for line_ending in ["\n", "\r\n", "\r"] {
            let source = format!("a{line_ending}b");
            let lexed = assert_lossless(&source, Options::CSV);
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
    }

    #[test]
    fn truncated_multibyte_and_deep_runs_stay_lossless() {
        assert_lossless("名前,\"未完", Options::CSV);
        assert_lossless("a,😀😀\n", Options::CSV);
        let many_quotes = format!("\"{}x", "\"\"".repeat(64));
        assert_lossless(&many_quotes, Options::CSV);
        let long_row = format!("{},last\n", "v,".repeat(300));
        assert_lossless(&long_row, Options::CSV);
    }

    #[test]
    fn every_truncation_rebuilds_its_prefix() {
        let sample = "h1,h2\n\"a,b\",\"c\r\nd\"\"e\",f\nx,y\"z,3\n";
        for options in [Options::CSV, Options::TSV] {
            for cut in 0..=sample.len() {
                assert_lossless(&sample[..cut], options);
            }
        }
    }
}
