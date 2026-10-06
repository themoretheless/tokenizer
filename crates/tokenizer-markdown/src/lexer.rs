//! A lossless, line-oriented CommonMark + GFM lexer.
//!
//! Concatenating token text reconstructs the source byte-for-byte, including
//! broken and incomplete documents: an abandoned construct keeps its own kind
//! and is flagged [`LexToken::has_error`] instead of being dropped or replaced
//! by a synthesized token. Markdown has no comment syntax, so nothing here
//! borrows a programming-language vocabulary.
//!
//! Block constructs are decided at the start of a line; inline constructs
//! (code spans, emphasis, links, raw tags) never cross a line break, which is
//! also what keeps their scans from backtracking across the document.

use themoretheless_tokenizer_core::Span;

/// Inline nesting explored before a region collapses to text. Unbalanced
/// brackets on one long line must not deepen the call stack.
const MAX_INLINE_DEPTH: usize = 16;

/// Exact lexical categories emitted by the Markdown lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxKind {
    HeadingMarker,
    HeadingText,
    SetextUnderline,
    ThematicBreak,
    BlockquoteMarker,
    ListMarker,
    CodeFenceMarker,
    FenceInfo,
    CodeBlockLine,
    TableDelimiter,
    TablePipe,
    TableCell,
    LinkLabel,
    LinkDestination,
    FrontMatterDelimiter,
    FrontMatter,
    HtmlBlock,
    HardBreak,
    LineBreak,
    Whitespace,

    Strong,
    Emphasis,
    Strikethrough,
    CodeSpan,
    LinkText,
    ImageMarker,
    Autolink,
    EmailAutolink,
    FootnoteRef,
    FootnoteDefinitionLabel,
    HtmlInline,
    Punctuation,
    Text,
    Error,
}

impl SyntaxKind {
    /// Trivia carries no Markdown structure.
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(self, Self::Whitespace | Self::LineBreak)
    }

    /// Kinds decided by a line's leading context rather than its content.
    #[must_use]
    pub const fn is_block_marker(self) -> bool {
        matches!(
            self,
            Self::HeadingMarker
                | Self::SetextUnderline
                | Self::ThematicBreak
                | Self::BlockquoteMarker
                | Self::ListMarker
                | Self::CodeFenceMarker
                | Self::TableDelimiter
                | Self::LinkLabel
                | Self::FootnoteDefinitionLabel
                | Self::HtmlBlock
                | Self::FrontMatterDelimiter
        )
    }
}

/// Compact per-token state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TokenFlags(u8);

impl TokenFlags {
    pub const EMPTY: Self = Self(0);
    pub const HAS_ERROR: Self = Self(1);

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    fn inserted(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// A lexical token. Spans are non-empty UTF-8 byte ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LexToken {
    pub kind: SyntaxKind,
    pub span: Span,
    pub flags: TokenFlags,
}

impl LexToken {
    #[must_use]
    pub fn text(self, source: &str) -> Option<&str> {
        source.get(self.span.start..self.span.end)
    }

    /// Whether this token is an abandoned construct. Its kind still names the
    /// construct, so the structural pass can report a specific code.
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

    /// Whether the token stream covers the source with no gaps or overlaps.
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        self.verify_lossless().is_ok()
    }

    /// Named lossless violation. Streams from [`lex`] always pass; the check is
    /// public so a host can re-verify a transformed token list.
    pub fn verify_lossless(&self) -> Result<(), themoretheless_tokenizer_core::LosslessViolation> {
        themoretheless_tokenizer_core::verify_lossless_spans(
            self.source,
            self.tokens.iter().map(|token| token.span),
        )
    }
}

/// Lexes Markdown.
#[must_use]
pub fn lex(source: &str) -> Lexed<'_> {
    let mut lexer = Lexer {
        source,
        bytes: source.as_bytes(),
        pos: 0,
        tokens: Vec::new(),
        depth: 0,
        open_fence: None,
        fence_token: None,
        table_state: TableState::None,
        in_html_block: false,
        prev: Prev::Blank,
    };
    lexer.run();
    Lexed {
        source,
        tokens: lexer.tokens,
    }
}

/// What the previous non-blank line was, which decides `---`/`===` readings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prev {
    Paragraph,
    Blank,
    Other,
}

/// Multi-line GFM table context: a header row promises a delimiter row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableState {
    None,
    DelimiterNext,
    Rows,
}

/// One line's byte regions. `body` is the trimmed content, `content_end`
/// excludes the line break, and `line_end` includes it.
#[derive(Debug, Clone, Copy)]
struct Line {
    start: usize,
    body_start: usize,
    body_end: usize,
    content_end: usize,
    line_end: usize,
}

impl Line {
    #[must_use]
    const fn blank(&self) -> bool {
        self.body_start == self.body_end
    }
}

/// A line's tail split: the inline region's end and any hard break after it.
#[derive(Debug, Clone, Copy)]
struct Tail {
    stop: usize,
    hard_break: Option<Span>,
}

/// Outcome of testing an inline delimiter run.
#[derive(Debug, Clone, Copy)]
enum InlineScan {
    /// Not an opener at all; continue from the offset with ordinary text.
    Literal(usize),
    /// A closed construct ending at the offset with the given kind.
    Closed(usize, SyntaxKind),
    /// An opener with no closer on the line: the region is abandoned.
    Abandoned(SyntaxKind),
}

struct Lexer<'source> {
    source: &'source str,
    bytes: &'source [u8],
    pos: usize,
    tokens: Vec<LexToken>,
    depth: usize,
    open_fence: Option<(u8, usize)>,
    fence_token: Option<usize>,
    table_state: TableState,
    in_html_block: bool,
    prev: Prev,
}

impl Lexer<'_> {
    fn run(&mut self) {
        self.lex_front_matter();
        while self.pos < self.bytes.len() {
            self.lex_line();
        }
        if let Some(index) = self.fence_token {
            self.mark_error(index);
        }
    }

    // ─── byte helpers ────────────────────────────────────────────────────────

    /// Byte at `i`, or `0` past the end. `0` never matches a structural byte.
    fn at(&self, i: usize) -> u8 {
        self.bytes.get(i).copied().unwrap_or(0)
    }

    fn has(&self, start: usize, end: usize, byte: u8) -> bool {
        self.bytes
            .get(start..end)
            .is_some_and(|slice| slice.contains(&byte))
    }

    /// UTF-8 width of the char starting at `i`; never straddles a boundary.
    fn char_len(&self, i: usize) -> usize {
        self.source
            .get(i..)
            .and_then(|slice| slice.chars().next())
            .map_or(1, char::len_utf8)
    }

    /// End of the run of `byte` starting at `i`, clamped to `to`.
    fn run_end(&self, i: usize, byte: u8, to: usize) -> usize {
        let mut j = i.min(to);
        while j < to && self.at(j) == byte {
            j += 1;
        }
        j
    }

    /// Shrinks `start..end` by leading and trailing spaces and tabs.
    fn trim(&self, start: usize, end: usize) -> (usize, usize) {
        let mut a = start;
        let mut b = end;
        while a < b && matches!(self.at(a), b' ' | b'\t') {
            a += 1;
        }
        while b > a && matches!(self.at(b - 1), b' ' | b'\t') {
            b -= 1;
        }
        (a, b)
    }

    /// `(content_end, line_end)` for the line at `start`. The content excludes
    /// its line break, including a `\r\n` pair or a lone `\r`.
    fn line_bounds(&self, start: usize) -> (usize, usize) {
        let mut i = start;
        while i < self.bytes.len() && self.bytes[i] != b'\n' && self.bytes[i] != b'\r' {
            i += 1;
        }
        let content_end = i;
        let mut line_end = i;
        if self.at(line_end) == b'\r' {
            line_end += 1;
            if self.at(line_end) == b'\n' {
                line_end += 1;
            }
        } else if self.at(line_end) == b'\n' {
            line_end += 1;
        }
        (content_end, line_end)
    }

    /// Space columns before `body_start`, where a tab advances to the next
    /// multiple of four.
    fn indent_columns(&self, start: usize, body_start: usize) -> usize {
        let mut columns = 0;
        let mut i = start;
        while i < body_start {
            columns = if self.at(i) == b'\t' {
                columns + (4 - columns % 4)
            } else {
                columns + 1
            };
            i += 1;
        }
        columns
    }

    fn line_at(&self, start: usize) -> Line {
        let (content_end, line_end) = self.line_bounds(start);
        let (body_start, body_end) = self.trim(start, content_end);
        Line {
            start,
            body_start,
            body_end,
            content_end,
            line_end,
        }
    }

    // ─── token output ────────────────────────────────────────────────────────

    /// Emits a token and returns its index. Zero-width spans are skipped so the
    /// stream never carries an empty token.
    fn push(&mut self, kind: SyntaxKind, start: usize, end: usize) -> Option<usize> {
        if end <= start {
            return None;
        }
        self.tokens.push(LexToken {
            kind,
            span: Span::new(start, end),
            flags: TokenFlags::EMPTY,
        });
        Some(self.tokens.len() - 1)
    }

    /// Emits an abandoned construct: its kind names it, its flag marks it.
    fn push_error(&mut self, kind: SyntaxKind, start: usize, end: usize) -> Option<usize> {
        if end <= start {
            return None;
        }
        self.tokens.push(LexToken {
            kind,
            span: Span::new(start, end),
            flags: TokenFlags::HAS_ERROR,
        });
        Some(self.tokens.len() - 1)
    }

    fn mark_error(&mut self, index: usize) {
        if let Some(token) = self.tokens.get_mut(index) {
            token.flags = token.flags.inserted(TokenFlags::HAS_ERROR);
        }
    }

    // ─── front matter ────────────────────────────────────────────────────────

    /// Start of the closing delimiter line, when the document opens with a
    /// `---` line at byte 0 and closes one at column 0. An unclosed `---` stays
    /// a thematic break instead.
    fn front_matter_close(&self) -> Option<usize> {
        if self.bytes.get(0..3) != Some(b"---") {
            return None;
        }
        let mut start = 0;
        let mut delimiters = 0;
        while start < self.bytes.len() {
            let line = self.line_at(start);
            if matches!(
                self.bytes.get(line.body_start..line.body_end),
                Some(b"---") | Some(b"...")
            ) {
                delimiters += 1;
                if delimiters == 2 {
                    return Some(line.start);
                }
            }
            start = line.line_end;
        }
        None
    }

    fn lex_front_matter(&mut self) {
        let Some(close) = self.front_matter_close() else {
            return;
        };
        let first = self.line_at(0);
        self.delimiter_line(first, SyntaxKind::FrontMatterDelimiter);
        while self.pos < close {
            let line = self.line_at(self.pos);
            self.push(SyntaxKind::Whitespace, line.start, line.body_start);
            self.push(SyntaxKind::FrontMatter, line.body_start, line.body_end);
            self.push(SyntaxKind::Whitespace, line.body_end, line.content_end);
            self.push_break(line);
            self.pos = line.line_end;
        }
        let last = self.line_at(self.pos);
        self.delimiter_line(last, SyntaxKind::FrontMatterDelimiter);
    }

    fn delimiter_line(&mut self, line: Line, kind: SyntaxKind) {
        self.push(SyntaxKind::Whitespace, line.start, line.body_start);
        self.push(kind, line.body_start, line.body_end);
        self.push(SyntaxKind::Whitespace, line.body_end, line.content_end);
        self.push_break(line);
        self.pos = line.line_end;
    }

    // ─── block level ─────────────────────────────────────────────────────────

    fn lex_line(&mut self) {
        let line = self.line_at(self.pos);
        if let Some((fence_char, fence_len)) = self.open_fence {
            self.lex_fenced_line(line, fence_char, fence_len);
            return;
        }
        if self.in_html_block {
            if line.blank() {
                self.in_html_block = false;
            } else {
                self.push(SyntaxKind::Whitespace, line.start, line.body_start);
                self.push(SyntaxKind::HtmlBlock, line.body_start, line.body_end);
                self.push(SyntaxKind::Whitespace, line.body_end, line.content_end);
                self.push_break(line);
                self.pos = line.line_end;
                return;
            }
        }
        if line.blank() {
            self.push(SyntaxKind::Whitespace, line.start, line.content_end);
            self.push_break(line);
            self.pos = line.line_end;
            self.prev = Prev::Blank;
            self.table_state = TableState::None;
            return;
        }

        let indent = self.indent_columns(line.start, line.body_start);
        let marker = self.at(line.body_start);
        let block_context = indent < 4;

        if block_context && matches!(marker, b'`' | b'~') && self.try_fence(line, marker) {
            return;
        }
        if block_context && marker == b'#' && self.try_heading(line) {
            return;
        }
        if block_context && self.try_thematic_or_setext(line, marker) {
            return;
        }
        if block_context && marker == b'>' {
            self.push(SyntaxKind::Whitespace, line.start, line.body_start);
            let run_end = self.run_end(line.body_start, b'>', line.content_end);
            self.push(SyntaxKind::BlockquoteMarker, line.body_start, run_end);
            self.prev = Prev::Other;
            self.lex_inline_tail(line, run_end);
            return;
        }
        if block_context {
            if let Some(marker_end) = self.list_marker_end(line) {
                self.push(SyntaxKind::Whitespace, line.start, line.body_start);
                self.push(SyntaxKind::ListMarker, line.body_start, marker_end);
                self.prev = Prev::Other;
                self.lex_inline_tail(line, marker_end);
                return;
            }
            if marker == b'['
                && let Some(label_end) = self.definition_label_end(line)
            {
                let footnote = self.at(line.body_start + 1) == b'^';
                self.push(SyntaxKind::Whitespace, line.start, line.body_start);
                self.push(
                    if footnote {
                        SyntaxKind::FootnoteDefinitionLabel
                    } else {
                        SyntaxKind::LinkLabel
                    },
                    line.body_start,
                    label_end,
                );
                self.prev = Prev::Other;
                self.lex_inline_tail(line, label_end);
                return;
            }
        }
        if self.try_table(line) {
            return;
        }
        if line.start == line.body_start
            && marker == b'<'
            && self.tag_name_end(line.body_start, line.body_end).is_some()
            && self.at(line.body_end - 1) == b'>'
        {
            self.push(SyntaxKind::HtmlBlock, line.body_start, line.body_end);
            self.push(SyntaxKind::Whitespace, line.body_end, line.content_end);
            self.push_break(line);
            self.pos = line.line_end;
            self.in_html_block = true;
            self.prev = Prev::Other;
            return;
        }

        self.push(SyntaxKind::Whitespace, line.start, line.body_start);
        self.prev = Prev::Paragraph;
        self.lex_inline_tail(line, line.body_start);
    }

    fn push_break(&mut self, line: Line) {
        self.push(SyntaxKind::LineBreak, line.content_end, line.line_end);
    }

    fn lex_fenced_line(&mut self, line: Line, fence_char: u8, fence_len: usize) {
        let close_run = self.run_end(line.body_start, fence_char, line.body_end);
        self.push(SyntaxKind::Whitespace, line.start, line.body_start);
        if close_run - line.body_start >= fence_len && close_run == line.body_end {
            self.push(SyntaxKind::CodeFenceMarker, line.body_start, close_run);
            self.open_fence = None;
            self.fence_token = None;
        } else {
            self.push(SyntaxKind::CodeBlockLine, line.body_start, line.body_end);
        }
        self.push(SyntaxKind::Whitespace, line.body_end, line.content_end);
        self.push_break(line);
        self.pos = line.line_end;
        self.prev = Prev::Other;
    }

    /// Emits a code-fence opener; the opener token is the handle later flagged
    /// when the document ends inside the fence.
    fn try_fence(&mut self, line: Line, marker: u8) -> bool {
        let run_end = self.run_end(line.body_start, marker, line.content_end);
        if run_end - line.body_start < 3 {
            return false;
        }
        let (info_start, info_end) = self.trim(run_end, line.body_end);
        // A backtick fence's info string may not contain a backtick.
        if marker == b'`' && self.has(info_start, info_end, b'`') {
            return false;
        }
        self.push(SyntaxKind::Whitespace, line.start, line.body_start);
        let marker_index = self.push(SyntaxKind::CodeFenceMarker, line.body_start, run_end);
        self.push(SyntaxKind::Whitespace, run_end, info_start);
        self.push(SyntaxKind::FenceInfo, info_start, info_end);
        self.push(SyntaxKind::Whitespace, info_end, line.content_end);
        self.push_break(line);
        self.pos = line.line_end;
        self.open_fence = Some((marker, run_end - line.body_start));
        self.fence_token = marker_index;
        self.prev = Prev::Other;
        true
    }

    /// Emits an ATX heading. A `#` run must be followed by whitespace or end
    /// the line, so `#hashtag` stays text. Seven or more `#` still emit a
    /// marker; the structural pass rejects its level.
    fn try_heading(&mut self, line: Line) -> bool {
        let run_end = self.run_end(line.body_start, b'#', line.content_end);
        if run_end < line.content_end && !matches!(self.at(run_end), b' ' | b'\t') {
            return false;
        }
        self.push(SyntaxKind::Whitespace, line.start, line.body_start);
        self.push(SyntaxKind::HeadingMarker, line.body_start, run_end);
        let (text_start, text_end) = self.trim(run_end, line.body_end);
        self.push(SyntaxKind::Whitespace, run_end, text_start);
        self.push(SyntaxKind::HeadingText, text_start, text_end);
        self.push(SyntaxKind::Whitespace, text_end, line.content_end);
        self.push_break(line);
        self.pos = line.line_end;
        self.prev = Prev::Other;
        true
    }

    /// A line of one repeated marker character is a setext underline after a
    /// paragraph, and otherwise a thematic break from three of them.
    fn try_thematic_or_setext(&mut self, line: Line, marker: u8) -> bool {
        if !matches!(marker, b'=' | b'-' | b'_' | b'*') {
            return false;
        }
        let mut count = 0;
        let mut i = line.body_start;
        while i < line.body_end {
            match self.at(i) {
                byte if byte == marker => count += 1,
                b' ' | b'\t' => {}
                _ => return false,
            }
            i += 1;
        }
        let paragraph = matches!(self.prev, Prev::Paragraph);
        let setext = paragraph && matches!(marker, b'=' | b'-');
        if !setext && (count < 3 || marker == b'=') {
            return false;
        }
        self.push(SyntaxKind::Whitespace, line.start, line.body_start);
        self.push(
            if setext {
                SyntaxKind::SetextUnderline
            } else {
                SyntaxKind::ThematicBreak
            },
            line.body_start,
            line.body_end,
        );
        self.push(SyntaxKind::Whitespace, line.body_end, line.content_end);
        self.push_break(line);
        self.pos = line.line_end;
        self.prev = Prev::Other;
        true
    }

    /// End of a bullet or ordered list marker opening `line`.
    fn list_marker_end(&self, line: Line) -> Option<usize> {
        let start = line.body_start;
        let end = line.body_end;
        if matches!(self.at(start), b'-' | b'+' | b'*') {
            let marker_end = start + 1;
            return (marker_end == end || matches!(self.at(marker_end), b' ' | b'\t'))
                .then_some(marker_end);
        }
        let mut i = start;
        while i < end && self.at(i).is_ascii_digit() {
            i += 1;
        }
        if i == start || i - start > 9 || !matches!(self.at(i), b'.' | b')') {
            return None;
        }
        let marker_end = i + 1;
        (marker_end == end || matches!(self.at(marker_end), b' ' | b'\t')).then_some(marker_end)
    }

    /// End of a `[label]:` or `[^label]:` definition marker opening `line`.
    fn definition_label_end(&self, line: Line) -> Option<usize> {
        let start = line.body_start;
        let end = line.body_end;
        let label_start = start + if self.at(start + 1) == b'^' { 2 } else { 1 };
        let mut i = label_start;
        while i < end && self.at(i) != b']' {
            if self.at(i) == b'[' {
                return None;
            }
            i += 1;
        }
        if i >= end || i == label_start || self.at(i + 1) != b':' {
            return None;
        }
        Some(i + 2)
    }

    /// Emits one line of a GFM table. A header row is only recognized together
    /// with the delimiter row that follows it.
    fn try_table(&mut self, line: Line) -> bool {
        if !self.has(line.body_start, line.body_end, b'|') {
            self.table_state = TableState::None;
            return false;
        }
        let delimiter_next = self.table_state == TableState::DelimiterNext;
        let in_rows = self.table_state == TableState::Rows;
        let starts_table = self.table_state == TableState::None
            && line.line_end < self.bytes.len()
            && self.is_delimiter_row(self.line_at(line.line_end));
        if !delimiter_next && !in_rows && !starts_table {
            return false;
        }
        self.push(SyntaxKind::Whitespace, line.start, line.body_start);
        if delimiter_next {
            self.push(SyntaxKind::TableDelimiter, line.body_start, line.body_end);
            self.table_state = TableState::Rows;
        } else {
            self.lex_table_row(line.body_start, line.body_end);
            if starts_table {
                self.table_state = TableState::DelimiterNext;
            }
        }
        self.push(SyntaxKind::Whitespace, line.body_end, line.content_end);
        self.push_break(line);
        self.pos = line.line_end;
        self.prev = Prev::Other;
        true
    }

    fn lex_table_row(&mut self, start: usize, end: usize) {
        let mut i = start;
        while i < end {
            match self.at(i) {
                b'|' => {
                    self.push(SyntaxKind::TablePipe, i, i + 1);
                    i += 1;
                }
                b' ' | b'\t' => {
                    let mut j = i;
                    while j < end && matches!(self.at(j), b' ' | b'\t') {
                        j += 1;
                    }
                    self.push(SyntaxKind::Whitespace, i, j);
                    i = j;
                }
                _ => {
                    let mut j = i;
                    while j < end && self.at(j) != b'|' {
                        j += self.char_len(j);
                    }
                    let (a, b) = self.trim(i, j);
                    self.push(SyntaxKind::Whitespace, i, a);
                    self.push(SyntaxKind::TableCell, a, b);
                    self.push(SyntaxKind::Whitespace, b, j);
                    i = j;
                }
            }
        }
    }

    fn is_delimiter_row(&self, line: Line) -> bool {
        if !self.has(line.body_start, line.body_end, b'|') {
            return false;
        }
        let mut cell_start = line.body_start;
        let mut i = line.body_start;
        let mut aligned = false;
        loop {
            if i == line.body_end || self.at(i) == b'|' {
                match self.delimiter_cell(cell_start, i) {
                    None => {}
                    Some(true) => aligned = true,
                    Some(false) => return false,
                }
                if i == line.body_end {
                    return aligned;
                }
                cell_start = i + 1;
            }
            i += 1;
        }
    }

    /// `None` for a blank cell, `Some(true)` for a `:---:` alignment cell.
    fn delimiter_cell(&self, start: usize, end: usize) -> Option<bool> {
        let (mut i, b) = self.trim(start, end);
        if i >= b {
            return None;
        }
        if self.at(i) == b':' {
            i += 1;
        }
        let dashes = self.run_end(i, b'-', b);
        if dashes == i {
            return Some(false);
        }
        i = dashes;
        if i < b && self.at(i) == b':' {
            i += 1;
        }
        Some(i == b)
    }

    /// End of a tag name after `<`, or `None` when the bytes after `<` are not
    /// a tag. `https:` and `a@b` fail here, so autolinks keep their own kinds.
    fn tag_name_end(&self, start: usize, end: usize) -> Option<usize> {
        let mut i = start + 1;
        if self.at(i) == b'/' {
            i += 1;
        }
        let name_start = i;
        while i < end && (self.at(i).is_ascii_alphanumeric() || self.at(i) == b'-') {
            i += 1;
        }
        if i == name_start {
            return None;
        }
        if i >= end {
            return Some(i);
        }
        matches!(self.at(i), b'>' | b' ' | b'\t' | b'/').then_some(i)
    }

    // ─── inline level ────────────────────────────────────────────────────────

    /// Lexes the line's remaining inline region, then its hard break and break.
    fn lex_inline_tail(&mut self, line: Line, from: usize) {
        let tail = self.split_tail(from, line.content_end);
        self.lex_inline(from, tail.stop);
        if let Some(span) = tail.hard_break {
            self.push(SyntaxKind::HardBreak, span.start, span.end);
        }
        self.push_break(line);
        self.pos = line.line_end;
    }

    /// Two trailing spaces or a trailing `\` immediately before the break are a
    /// hard break; a single trailing space stays whitespace inside the region.
    fn split_tail(&self, start: usize, end: usize) -> Tail {
        if end > start && self.at(end - 1) == b'\\' {
            return Tail {
                stop: end - 1,
                hard_break: Some(Span::new(end - 1, end)),
            };
        }
        let mut blank = end;
        while blank > start && matches!(self.at(blank - 1), b' ' | b'\t') {
            blank -= 1;
        }
        if end - blank >= 2 && blank > start {
            return Tail {
                stop: blank,
                hard_break: Some(Span::new(blank, end)),
            };
        }
        Tail {
            stop: end,
            hard_break: None,
        }
    }

    fn lex_inline(&mut self, from: usize, to: usize) {
        if from >= to {
            return;
        }
        if self.depth >= MAX_INLINE_DEPTH {
            self.push(SyntaxKind::Text, from, to);
            return;
        }
        self.depth += 1;
        let mut i = from;
        let mut run = from;
        while i < to {
            match self.at(i) {
                b' ' | b'\t' => {
                    self.push(SyntaxKind::Text, run, i);
                    let mut j = i;
                    while j < to && matches!(self.at(j), b' ' | b'\t') {
                        j += 1;
                    }
                    self.push(SyntaxKind::Whitespace, i, j);
                    i = j;
                    run = j;
                }
                b'`' => {
                    self.push(SyntaxKind::Text, run, i);
                    if let Some(end) = self.code_span_end(i, to) {
                        self.push(SyntaxKind::CodeSpan, i, end);
                        i = end;
                    } else {
                        self.push_error(SyntaxKind::CodeSpan, i, to);
                        i = to;
                    }
                    run = i;
                }
                b'*' | b'_' => {
                    let scan = self.emphasis_scan(from, i, to);
                    (i, run) = self.apply_scan(run, i, to, scan);
                }
                b'~' => {
                    let scan = self.strike_scan(i, to);
                    (i, run) = self.apply_scan(run, i, to, scan);
                }
                b'[' => {
                    self.push(SyntaxKind::Text, run, i);
                    if let Some(next) = self.try_link(i, to) {
                        i = next;
                    } else {
                        self.push(SyntaxKind::Punctuation, i, i + 1);
                        i += 1;
                    }
                    run = i;
                }
                b']' | b'(' | b')' => {
                    self.push(SyntaxKind::Text, run, i);
                    self.push(SyntaxKind::Punctuation, i, i + 1);
                    i += 1;
                    run = i;
                }
                b'!' if self.at(i + 1) == b'[' => {
                    self.push(SyntaxKind::Text, run, i);
                    self.push(SyntaxKind::ImageMarker, i, i + 1);
                    i += 1;
                    run = i;
                }
                b'<' => {
                    let scan = self.angle_scan(i, to);
                    (i, run) = self.apply_scan(run, i, to, scan);
                }
                _ => {
                    i += self.char_len(i);
                }
            }
        }
        self.push(SyntaxKind::Text, run, to);
        self.depth -= 1;
    }

    /// A code span closes on the next backtick run of equal length, on this
    /// line only.
    fn code_span_end(&self, start: usize, to: usize) -> Option<usize> {
        let open = self.run_end(start, b'`', to);
        let len = open - start;
        let mut i = open;
        while i < to {
            if self.at(i) == b'`' {
                let end = self.run_end(i, b'`', to);
                if end - i == len {
                    return Some(end);
                }
                i = end;
            } else {
                i += 1;
            }
        }
        None
    }

    /// Strong for a two-byte opener, emphasis for a single one. The whole span
    /// including its markers is one token, so nested markers stay ordinary text.
    fn emphasis_scan(&self, from: usize, start: usize, to: usize) -> InlineScan {
        let byte = self.at(start);
        let open_end = self.run_end(start, byte, to);
        let need = if open_end - start >= 2 { 2 } else { 1 };
        let kind = if need == 2 {
            SyntaxKind::Strong
        } else {
            SyntaxKind::Emphasis
        };
        if open_end >= to || matches!(self.at(open_end), b' ' | b'\t') {
            return InlineScan::Literal(open_end);
        }
        if byte == b'_' && start > from && self.at(start - 1).is_ascii_alphanumeric() {
            return InlineScan::Literal(open_end);
        }
        let mut j = open_end;
        while j < to {
            if self.at(j) != byte {
                j += 1;
                continue;
            }
            let close_end = self.run_end(j, byte, to);
            if close_end - j >= need && close_end > open_end {
                let before_ok = !matches!(self.at(j - 1), b' ' | b'\t');
                let after_ok =
                    byte != b'_' || close_end >= to || !self.at(close_end).is_ascii_alphanumeric();
                if before_ok && after_ok {
                    return InlineScan::Closed(close_end, kind);
                }
            }
            j = close_end;
        }
        InlineScan::Abandoned(kind)
    }

    fn strike_scan(&self, start: usize, to: usize) -> InlineScan {
        let open_end = self.run_end(start, b'~', to);
        if open_end - start < 2 {
            return InlineScan::Literal(open_end);
        }
        let scan = self.emphasis_scan(start, start, to);
        match scan {
            InlineScan::Literal(offset) => InlineScan::Literal(offset),
            InlineScan::Closed(end, _) => InlineScan::Closed(end, SyntaxKind::Strikethrough),
            InlineScan::Abandoned(_) => InlineScan::Abandoned(SyntaxKind::Strikethrough),
        }
    }

    /// Emits a scanned construct and returns the `(offset, text-run)` pair to
    /// continue inline lexing from.
    fn apply_scan(
        &mut self,
        run: usize,
        start: usize,
        to: usize,
        scan: InlineScan,
    ) -> (usize, usize) {
        match scan {
            InlineScan::Literal(next) => (next, run),
            InlineScan::Closed(end, kind) => {
                self.push(SyntaxKind::Text, run, start);
                self.push(kind, start, end);
                (end, end)
            }
            InlineScan::Abandoned(kind) => {
                self.push(SyntaxKind::Text, run, start);
                self.push_error(kind, start, to);
                (to, to)
            }
        }
    }

    /// Offset after a link, image, or footnote reference, emitting its parts;
    /// `None` means the `[` is bare punctuation.
    fn try_link(&mut self, start: usize, to: usize) -> Option<usize> {
        let footnote = self.at(start + 1) == b'^';
        let close = self.bracket_close(start, to)?;
        if footnote {
            if close == start + 2 {
                return None;
            }
            self.push(SyntaxKind::FootnoteRef, start, close + 1);
            return Some(close + 1);
        }
        match self.at(close + 1) {
            b'(' => {
                self.push(SyntaxKind::LinkText, start, close + 1);
                let open = close + 1;
                if let Some(end) = self.byte_close(open + 1, to, b')') {
                    self.push(SyntaxKind::LinkDestination, open, end + 1);
                    Some(end + 1)
                } else {
                    self.push_error(SyntaxKind::LinkDestination, open, to);
                    Some(to)
                }
            }
            b'[' => match self.bracket_close(close + 1, to) {
                Some(second) => {
                    self.push(SyntaxKind::LinkText, start, close + 1);
                    self.push(SyntaxKind::LinkText, close + 1, second + 1);
                    Some(second + 1)
                }
                None => None,
            },
            _ => {
                self.push(SyntaxKind::Punctuation, start, start + 1);
                self.lex_inline(start + 1, close);
                self.push(SyntaxKind::Punctuation, close, close + 1);
                Some(close + 1)
            }
        }
    }

    /// Index of the `]` closing the `[` at `start`, rejecting nested brackets.
    fn bracket_close(&self, start: usize, to: usize) -> Option<usize> {
        let mut i = start + 1;
        while i < to {
            match self.at(i) {
                b']' => return Some(i),
                b'[' => return None,
                _ => i += self.char_len(i),
            }
        }
        None
    }

    fn byte_close(&self, start: usize, to: usize, byte: u8) -> Option<usize> {
        let mut i = start;
        while i < to {
            if self.at(i) == byte {
                return Some(i);
            }
            i += self.char_len(i);
        }
        None
    }

    /// Autolink, email autolink, or inline raw tag. `Literal` means the `<` is
    /// ordinary text and scanning resumes just after it.
    fn angle_scan(&self, start: usize, to: usize) -> InlineScan {
        let tag = self.tag_name_end(start, to).is_some();
        let mut i = start + 1;
        let mut spaced = false;
        while i < to {
            match self.at(i) {
                b'>' => {
                    if !spaced && i > start + 1 {
                        let inner = self.bytes.get(start + 1..i).unwrap_or_default();
                        if inner.contains(&b':') {
                            return InlineScan::Closed(i + 1, SyntaxKind::Autolink);
                        }
                        if inner.contains(&b'@') {
                            return InlineScan::Closed(i + 1, SyntaxKind::EmailAutolink);
                        }
                    }
                    if tag {
                        return InlineScan::Closed(i + 1, SyntaxKind::HtmlInline);
                    }
                    break;
                }
                b' ' | b'\t' => spaced = true,
                b'<' => break,
                _ => {}
            }
            i += 1;
        }
        if tag && i >= to {
            return InlineScan::Abandoned(SyntaxKind::HtmlInline);
        }
        InlineScan::Literal(start + 1)
    }
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
        let mut previous = 0;
        for token in lexed.tokens() {
            assert!(!token.span.is_empty(), "{source:?}");
            assert!(token.span.start >= previous, "{source:?} overlaps");
            assert!(source.is_char_boundary(token.span.start), "{source:?}");
            assert!(source.is_char_boundary(token.span.end), "{source:?}");
            previous = token.span.end;
        }
        lexed
    }

    #[test]
    fn corpus_is_lossless() {
        let corpus = [
            "# Title\n\nSome **bold**, *em* and `code` in a paragraph.\n",
            "- item one\n- [link](https://example.com)\n\n> quoted\n",
            "```rust\nfn main() {}\n```\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n",
            "See [^1].\n\n[^1]: note\n\n[ref]: /url \"title\"\n",
            "---\ntitle: Markdown\ntags:\n  - one\n---\n\n# After front matter\n",
            "Привет **мир** 🎉 *акцент* <https://пример.рф> <mail@example.com>\n",
            "first line\r\nsecond  \r\n- [x] task\r\n\r\n~~struck~~ and <b>tag</b>\r\n",
            "\t- tab item\n\t\tdeeper\n* star item\n+ plus item\n1. one\n2) two\n",
            "# unclosed fence\n\n```js\nconst x = 1\n",
            "Some **bold and a dangling `span\n\n~~unclosed\n\n<a href=\"x\">\n",
            "text ends with backslash\\\na\\\n",
            "####### seven hashes\n",
            "#",
            "`",
            "[link](",
            "",
            "\n\n\n",
            "  ",
            "a | b\n--- | ---\nc | d\n",
            "<div>\n**not emphasis**\n</div>\n",
        ];
        for source in corpus {
            assert_lossless(source);
        }
    }

    #[test]
    fn heading_marker_is_not_a_comment() {
        assert_eq!(
            kinds("# Title\n"),
            vec![
                SyntaxKind::HeadingMarker,
                SyntaxKind::Whitespace,
                SyntaxKind::HeadingText,
                SyntaxKind::LineBreak
            ]
        );
    }

    #[test]
    fn hash_without_space_or_seven_hashes() {
        assert_eq!(
            kinds("#hashtag\n"),
            vec![SyntaxKind::Text, SyntaxKind::LineBreak]
        );
        let lexed = assert_lossless("####### seven\n");
        assert_eq!(lexed.tokens()[0].kind, SyntaxKind::HeadingMarker);
        assert_eq!(lexed.tokens()[0].span.len(), 7);
        assert!(!lexed.tokens()[0].has_error());
    }

    #[test]
    fn setext_underline_beats_thematic_break_after_paragraph() {
        assert!(kinds("Title\n---\n").contains(&SyntaxKind::SetextUnderline));
        assert!(kinds("Title\n===\n").contains(&SyntaxKind::SetextUnderline));
        assert!(kinds("# h\n\n---\n\n").contains(&SyntaxKind::ThematicBreak));
        assert!(kinds("- - -\n").contains(&SyntaxKind::ThematicBreak));
    }

    #[test]
    fn star_at_line_start_is_a_list_marker() {
        assert_eq!(
            kinds("* item\n"),
            vec![
                SyntaxKind::ListMarker,
                SyntaxKind::Whitespace,
                SyntaxKind::Text,
                SyntaxKind::LineBreak
            ]
        );
        assert_eq!(
            kinds("**bold**\n"),
            vec![SyntaxKind::Strong, SyntaxKind::LineBreak]
        );
    }

    #[test]
    fn fenced_body_is_not_lexed_inline() {
        let lexed = assert_lossless("```rust\nlet x = 1; // not a comment\n```\n");
        let body = lexed
            .tokens()
            .iter()
            .find(|token| token.kind == SyntaxKind::CodeBlockLine)
            .copied()
            .expect("code line");
        assert_eq!(
            body.text(lexed.source()),
            Some("let x = 1; // not a comment")
        );
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::CodeFenceMarker)
                .count(),
            2
        );
    }

    #[test]
    fn unclosed_inline_constructs_are_flagged() {
        for source in [
            "**bold\n",
            "*em\n",
            "`code\n",
            "~~s\n",
            "<b attr\n",
            "[a](b\n",
            "text **more\n",
        ] {
            let lexed = assert_lossless(source);
            assert!(lexed.has_errors(), "{source:?}");
        }
    }

    #[test]
    fn unclosed_fence_flags_its_opener() {
        let lexed = assert_lossless("```\nbody\n");
        let marker = lexed
            .tokens()
            .iter()
            .find(|token| token.kind == SyntaxKind::CodeFenceMarker)
            .copied()
            .expect("opening fence");
        assert!(marker.has_error());
    }

    #[test]
    fn inline_constructs_are_whole_spans() {
        assert_eq!(
            kinds("a **b** c *d* e `f` g ~~h~~ i\n"),
            vec![
                SyntaxKind::Text,
                SyntaxKind::Whitespace,
                SyntaxKind::Strong,
                SyntaxKind::Whitespace,
                SyntaxKind::Text,
                SyntaxKind::Whitespace,
                SyntaxKind::Emphasis,
                SyntaxKind::Whitespace,
                SyntaxKind::Text,
                SyntaxKind::Whitespace,
                SyntaxKind::CodeSpan,
                SyntaxKind::Whitespace,
                SyntaxKind::Text,
                SyntaxKind::Whitespace,
                SyntaxKind::Strikethrough,
                SyntaxKind::Whitespace,
                SyntaxKind::Text,
                SyntaxKind::LineBreak,
            ]
        );
        assert_eq!(
            kinds("`` a ` b ``\n"),
            vec![SyntaxKind::CodeSpan, SyntaxKind::LineBreak]
        );
    }

    #[test]
    fn snake_case_underscores_stay_text() {
        assert_eq!(
            kinds("snake_case_name\n"),
            vec![SyntaxKind::Text, SyntaxKind::LineBreak]
        );
        assert_eq!(
            kinds("_real_ emphasis\n"),
            vec![
                SyntaxKind::Emphasis,
                SyntaxKind::Whitespace,
                SyntaxKind::Text,
                SyntaxKind::LineBreak
            ]
        );
    }

    #[test]
    fn links_images_and_autolinks() {
        let lexed = assert_lossless("![alt](a.png) and [x](y) and f(x) <https://z> <a@b.c>\n");
        for kind in [
            SyntaxKind::ImageMarker,
            SyntaxKind::LinkText,
            SyntaxKind::LinkDestination,
            SyntaxKind::Autolink,
            SyntaxKind::EmailAutolink,
        ] {
            assert!(
                lexed.tokens().iter().any(|token| token.kind == kind),
                "missing {kind:?}"
            );
        }
        assert!(
            lexed
                .tokens()
                .iter()
                .any(|token| token.kind == SyntaxKind::Punctuation)
        );
    }

    #[test]
    fn hard_breaks_and_crlf() {
        let lexed = assert_lossless("one  \ntwo\\\r\n");
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::HardBreak)
                .count(),
            2
        );
        assert!(
            lexed
                .tokens()
                .iter()
                .any(|token| token.text(lexed.source()) == Some("\r\n"))
        );
    }

    #[test]
    fn definition_lines_label_their_prefix() {
        assert!(kinds("[ref]: https://example.com\n").contains(&SyntaxKind::LinkLabel));
        assert!(kinds("[^1]: note\n").contains(&SyntaxKind::FootnoteDefinitionLabel));
        assert!(kinds("See [^1].\n").contains(&SyntaxKind::FootnoteRef));
    }

    #[test]
    fn table_rows_use_pipe_and_cell_kinds() {
        let lexed = assert_lossless("| a | b |\n| --- | --- |\n| 1 | 2 |\n");
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::TablePipe)
                .count(),
            6
        );
        assert!(
            lexed
                .tokens()
                .iter()
                .any(|token| token.kind == SyntaxKind::TableDelimiter)
        );
    }

    #[test]
    fn tabs_and_deep_indent_never_panic() {
        assert_lossless("\t- item\n\t\tdeeper\n");
        assert_lossless("    # not a heading\n");
        assert_lossless("\t\t\t");
        assert_lossless("#\t\t\n");
    }

    #[test]
    fn truncated_multibyte_input_stays_lossless() {
        assert_lossless("# Три заголовка 😀未完");
        assert_lossless("[link](https://ex");
        assert_lossless("`код");
        assert_lossless("**жирный");
        assert_lossless("<https://пример");
        assert_lossless("emoji 😀 tail");
    }

    #[test]
    fn empty_and_whitespace_only_input() {
        assert_eq!(kinds(""), Vec::<SyntaxKind>::new());
        assert_lossless("");
        assert_lossless("   ");
        assert_lossless("\n");
    }

    #[test]
    fn deep_bracket_nesting_stays_lossless() {
        let source = format!("{}x{}", "[".repeat(200), "]".repeat(200));
        assert_lossless(&source);
    }

    #[test]
    fn html_block_swallows_markdown_until_a_blank_line() {
        let lexed = assert_lossless("<div>\n**not emphasis**\n</div>\n\nafter\n");
        assert_eq!(
            lexed
                .tokens()
                .iter()
                .filter(|token| token.kind == SyntaxKind::HtmlBlock)
                .count(),
            3
        );
        assert!(!kinds("after\n").contains(&SyntaxKind::HtmlBlock));
    }
}
