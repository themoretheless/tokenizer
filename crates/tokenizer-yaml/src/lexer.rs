//! A lossless, single-pass YAML 1.2 lexer.
//!
//! Token text concatenation always reconstructs the source byte-for-byte,
//! including malformed and incomplete editor text. Line breaks are significant
//! tokens because YAML uses line structure and indentation to delimit nodes;
//! the parser derives columns from token spans instead of the lexer keeping an
//! indentation stack. Block scalar content is therefore emitted as ordinary
//! scalar and structural tokens after a [`SyntaxKind::BlockScalarHeader`];
//! the parser owns dedent detection and folding.

use std::{error::Error, fmt};

use crate::Span;

/// Exact lexical categories emitted by the YAML lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxKind {
    Whitespace,
    LineBreak,
    Bom,
    Comment,
    DocumentStart,
    DocumentEnd,
    Directive,
    BlockEntry,
    KeyIndicator,
    ValueIndicator,
    FlowSequenceStart,
    FlowSequenceEnd,
    FlowMappingStart,
    FlowMappingEnd,
    FlowEntry,
    Anchor,
    Alias,
    Tag,
    BlockScalarHeader,
    SingleQuotedScalar,
    DoubleQuotedScalar,
    PlainScalar,
    Error,
}

impl SyntaxKind {
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(self, Self::Whitespace | Self::Bom | Self::Comment)
    }

    /// Scalars and node-introducing indicators that can begin a YAML node.
    #[must_use]
    pub const fn can_start_node(self) -> bool {
        matches!(
            self,
            Self::SingleQuotedScalar
                | Self::DoubleQuotedScalar
                | Self::PlainScalar
                | Self::FlowSequenceStart
                | Self::FlowMappingStart
                | Self::BlockScalarHeader
                | Self::Anchor
                | Self::Tag
                | Self::Alias
        )
    }
}

/// Compact per-token state that survives diagnostic truncation.
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
}

/// A lexical token. Spans are non-empty UTF-8 byte ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
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

    #[must_use]
    pub const fn has_error(self) -> bool {
        self.flags.contains(TokenFlags::HAS_ERROR)
    }
}

/// The category of a lexical diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LexDiagnosticKind {
    UnexpectedCharacter,
    TabInIndentation,
    InvalidEscape,
    InvalidUnicodeEscape,
    UnescapedControlCharacter,
    UnterminatedQuotedScalar,
    InvalidAnchorName,
    InvalidTag,
    InvalidDirective,
    InvalidBlockScalarHeader,
    InputLimitExceeded,
    TokenLimitExceeded,
    TooManyDiagnostics,
}

impl LexDiagnosticKind {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnexpectedCharacter => "unexpected-character",
            Self::TabInIndentation => "tab-in-indentation",
            Self::InvalidEscape => "invalid-escape",
            Self::InvalidUnicodeEscape => "invalid-unicode-escape",
            Self::UnescapedControlCharacter => "unescaped-control-character",
            Self::UnterminatedQuotedScalar => "unterminated-quoted-scalar",
            Self::InvalidAnchorName => "invalid-anchor-name",
            Self::InvalidTag => "invalid-tag",
            Self::InvalidDirective => "invalid-directive",
            Self::InvalidBlockScalarHeader => "invalid-block-scalar-header",
            Self::InputLimitExceeded => "input-limit-exceeded",
            Self::TokenLimitExceeded => "token-limit-exceeded",
            Self::TooManyDiagnostics => "too-many-lex-diagnostics",
        }
    }
}

impl fmt::Display for LexDiagnosticKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnexpectedCharacter => "unexpected character in YAML",
            Self::TabInIndentation => "tabs cannot be used for YAML indentation",
            Self::InvalidEscape => "invalid YAML double-quoted escape",
            Self::InvalidUnicodeEscape => {
                "expected two, four or eight hexadecimal digits after `\\x`, `\\u` or `\\U`"
            }
            Self::UnescapedControlCharacter => {
                "YAML scalars and comments cannot contain control characters other than tab"
            }
            Self::UnterminatedQuotedScalar => "unterminated YAML quoted scalar",
            Self::InvalidAnchorName => "expected an anchor or alias name after `&` or `*`",
            Self::InvalidTag => "invalid YAML tag",
            Self::InvalidDirective => "invalid YAML directive",
            Self::InvalidBlockScalarHeader => "invalid YAML block scalar header",
            Self::InputLimitExceeded => "the configured YAML input byte limit was exceeded",
            Self::TokenLimitExceeded => "the configured YAML token limit was exceeded",
            Self::TooManyDiagnostics => "additional lexical diagnostics were omitted",
        };
        formatter.write_str(message)
    }
}

/// A lexical problem with an exact source range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct LexDiagnostic {
    pub kind: LexDiagnosticKind,
    pub span: Span,
}

impl fmt::Display for LexDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}..{}",
            self.kind, self.span.start, self.span.end
        )
    }
}

impl Error for LexDiagnostic {}

/// Lexer configuration. Builders keep future additions backward-compatible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LexerOptions {
    max_input_bytes: usize,
    max_tokens: usize,
    max_diagnostics: usize,
}

impl LexerOptions {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_input_bytes: 16 * 1024 * 1024,
            max_tokens: 1_000_000,
            max_diagnostics: 256,
        }
    }

    #[must_use]
    pub const fn max_input_bytes(mut self, max_input_bytes: usize) -> Self {
        self.max_input_bytes = max_input_bytes;
        self
    }

    #[must_use]
    pub const fn max_tokens(mut self, max_tokens: usize) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    #[must_use]
    pub const fn max_diagnostics(mut self, max_diagnostics: usize) -> Self {
        self.max_diagnostics = max_diagnostics;
        self
    }

    #[must_use]
    pub const fn input_byte_limit(self) -> usize {
        self.max_input_bytes
    }

    #[must_use]
    pub const fn token_limit(self) -> usize {
        self.max_tokens
    }

    #[must_use]
    pub const fn diagnostic_limit(self) -> usize {
        self.max_diagnostics
    }
}

impl Default for LexerOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Lossless lexer output. Concatenating token text always reconstructs the
/// original source byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lexed<'source> {
    source: &'source str,
    tokens: Vec<LexToken>,
    diagnostics: Vec<LexDiagnostic>,
    truncated: bool,
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

    #[must_use]
    pub fn diagnostics(&self) -> &[LexDiagnostic] {
        &self.diagnostics
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
        !self.diagnostics.is_empty()
    }

    #[must_use]
    pub const fn is_truncated(&self) -> bool {
        self.truncated
    }
}

/// Lexes YAML.
#[must_use]
pub fn lex(source: &str) -> Lexed<'_> {
    lex_with(source, LexerOptions::new())
}

/// Lexes YAML using the supplied options.
#[must_use]
pub fn lex_with(source: &str, options: LexerOptions) -> Lexed<'_> {
    if source.len() > options.max_input_bytes {
        let span = Span::new(0, source.len());
        return Lexed {
            source,
            tokens: vec![LexToken {
                kind: SyntaxKind::Error,
                span,
                flags: TokenFlags::HAS_ERROR,
            }],
            diagnostics: vec![LexDiagnostic {
                kind: LexDiagnosticKind::InputLimitExceeded,
                span,
            }],
            truncated: true,
        };
    }
    Lexer {
        source,
        options,
        cursor: 0,
        tokens: Vec::new(),
        diagnostics: Vec::new(),
        diagnostics_truncated: false,
        truncated: false,
        line_start_offset: 0,
        line_has_content: false,
        flow_depth: 0,
    }
    .run()
}

struct Lexer<'source> {
    source: &'source str,
    options: LexerOptions,
    cursor: usize,
    tokens: Vec<LexToken>,
    diagnostics: Vec<LexDiagnostic>,
    diagnostics_truncated: bool,
    truncated: bool,
    line_start_offset: usize,
    line_has_content: bool,
    flow_depth: usize,
}

impl<'source> Lexer<'source> {
    fn run(mut self) -> Lexed<'source> {
        while self.cursor < self.source.len() {
            if self.tokens.len() >= self.options.max_tokens {
                let start = self.cursor;
                self.cursor = self.source.len();
                self.push(SyntaxKind::Error, start, self.cursor);
                self.problem(LexDiagnosticKind::TokenLimitExceeded, start, self.cursor);
                self.truncated = true;
                break;
            }
            let start = self.cursor;
            match self.bytes()[start] {
                b' ' | b'\t' => self.scan_whitespace(start),
                b'\n' | b'\r' => self.scan_line_break(start),
                b'#' => self.scan_comment(start),
                b'[' => self.flow_token(SyntaxKind::FlowSequenceStart),
                b']' => self.flow_token(SyntaxKind::FlowSequenceEnd),
                b'{' => self.flow_token(SyntaxKind::FlowMappingStart),
                b'}' => self.flow_token(SyntaxKind::FlowMappingEnd),
                b',' => self.single(SyntaxKind::FlowEntry),
                b':' => self.scan_colon(start),
                b'-' => self.scan_minus(start),
                b'.' => self.scan_dot(start),
                b'?' => self.scan_question(start),
                b'&' => self.scan_anchor(SyntaxKind::Anchor, start),
                b'*' => self.scan_anchor(SyntaxKind::Alias, start),
                b'!' => self.scan_tag(start),
                b'%' => self.scan_percent(start),
                b'|' | b'>' if self.flow_depth == 0 => self.scan_block_scalar_header(start),
                b'"' => self.scan_double_quoted(start),
                b'\'' => self.scan_single_quoted(start),
                _ if start == 0 && self.source.starts_with('\u{feff}') => {
                    self.cursor += '\u{feff}'.len_utf8();
                    self.push(SyntaxKind::Bom, start, self.cursor);
                }
                _ => self.scan_atom(start),
            }
            debug_assert!(self.cursor > start, "the lexer must always make progress");
        }

        self.diagnostics
            .sort_by_key(|diagnostic| (diagnostic.span.start, diagnostic.span.end));
        Lexed {
            source: self.source,
            tokens: self.tokens,
            diagnostics: self.diagnostics,
            truncated: self.truncated || self.diagnostics_truncated,
        }
    }

    fn bytes(&self) -> &[u8] {
        self.source.as_bytes()
    }

    fn single(&mut self, kind: SyntaxKind) {
        let start = self.cursor;
        self.cursor += 1;
        self.push(kind, start, self.cursor);
    }

    fn flow_token(&mut self, kind: SyntaxKind) {
        match kind {
            SyntaxKind::FlowSequenceStart | SyntaxKind::FlowMappingStart => self.flow_depth += 1,
            SyntaxKind::FlowSequenceEnd | SyntaxKind::FlowMappingEnd => {
                self.flow_depth = self.flow_depth.saturating_sub(1);
            }
            _ => {}
        }
        self.single(kind);
    }

    fn scan_whitespace(&mut self, start: usize) {
        let mut saw_tab = false;
        while let Some(&byte) = self.bytes().get(self.cursor) {
            match byte {
                b' ' => self.cursor += 1,
                b'\t' => {
                    saw_tab = true;
                    self.cursor += 1;
                }
                _ => break,
            }
        }
        if !self.line_has_content && saw_tab {
            self.problem(LexDiagnosticKind::TabInIndentation, start, self.cursor);
        }
        self.push(SyntaxKind::Whitespace, start, self.cursor);
    }

    fn scan_line_break(&mut self, start: usize) {
        self.cursor += 1;
        if self.bytes()[start] == b'\r' && self.bytes().get(self.cursor) == Some(&b'\n') {
            self.cursor += 1;
        }
        self.push(SyntaxKind::LineBreak, start, self.cursor);
    }

    /// Consumes one YAML line break (LF, CRLF, or lone CR) inside a scanner
    /// that continues past the end of the line.
    fn consume_break(&mut self) {
        let byte = self.bytes()[self.cursor];
        self.cursor += 1;
        if byte == b'\r' && self.bytes().get(self.cursor) == Some(&b'\n') {
            self.cursor += 1;
        }
    }

    fn scan_comment(&mut self, start: usize) {
        let mut valid = true;
        self.cursor += 1;
        while self.cursor < self.source.len() {
            match self.bytes()[self.cursor] {
                b'\r' | b'\n' => break,
                byte if is_forbidden_control(byte) => {
                    let error_start = self.cursor;
                    self.cursor += 1;
                    valid = false;
                    self.problem(
                        LexDiagnosticKind::UnescapedControlCharacter,
                        error_start,
                        self.cursor,
                    );
                }
                _ => self.cursor = next_boundary(self.source, self.cursor),
            }
        }
        self.push(SyntaxKind::Comment, start, self.cursor);
        if !valid {
            self.mark_last_error();
        }
    }

    fn scan_colon(&mut self, start: usize) {
        let next = self.bytes().get(start + 1).copied();
        let before_blank = next.is_none_or(is_blank_or_break);
        let before_flow = self.flow_depth > 0
            && next.is_some_and(|byte| matches!(byte, b',' | b'[' | b']' | b'{' | b'}'));
        if before_blank || before_flow {
            self.single(SyntaxKind::ValueIndicator);
        } else {
            self.scan_plain(start);
        }
    }

    fn scan_minus(&mut self, start: usize) {
        if start == self.line_start_offset
            && self.matches_at(start, b"---")
            && self.blank_or_break_at(start + 3)
        {
            self.cursor = start + 3;
            self.push(SyntaxKind::DocumentStart, start, self.cursor);
        } else if !self.line_has_content
            && self.flow_depth == 0
            && self.blank_or_break_at(start + 1)
        {
            self.single(SyntaxKind::BlockEntry);
        } else {
            self.scan_plain(start);
        }
    }

    fn scan_dot(&mut self, start: usize) {
        if start == self.line_start_offset
            && self.matches_at(start, b"...")
            && self.blank_or_break_at(start + 3)
        {
            self.cursor = start + 3;
            self.push(SyntaxKind::DocumentEnd, start, self.cursor);
        } else {
            self.scan_plain(start);
        }
    }

    fn scan_question(&mut self, start: usize) {
        if self.blank_or_break_at(start + 1) {
            self.single(SyntaxKind::KeyIndicator);
        } else {
            self.scan_plain(start);
        }
    }

    fn scan_percent(&mut self, start: usize) {
        if start == self.line_start_offset && self.flow_depth == 0 && !self.line_has_content {
            self.scan_directive(start);
        } else {
            self.scan_plain(start);
        }
    }

    fn scan_directive(&mut self, start: usize) {
        let mut valid = true;
        self.cursor = start + 1;
        while self.cursor < self.source.len() {
            match self.bytes()[self.cursor] {
                b'\r' | b'\n' => break,
                byte if is_forbidden_control(byte) => {
                    let error_start = self.cursor;
                    self.cursor += 1;
                    valid = false;
                    self.problem(
                        LexDiagnosticKind::InvalidDirective,
                        error_start,
                        self.cursor,
                    );
                }
                _ => self.cursor = next_boundary(self.source, self.cursor),
            }
        }
        self.push(SyntaxKind::Directive, start, self.cursor);
        if !valid {
            self.mark_last_error();
        }
    }

    fn scan_block_scalar_header(&mut self, start: usize) {
        let mut valid = true;
        let mut seen_digit = false;
        let mut seen_chomp = false;
        self.cursor = start + 1;
        while self.cursor < self.source.len() {
            match self.bytes()[self.cursor] {
                digit @ b'0'..=b'9' => {
                    if seen_digit || digit == b'0' {
                        valid = false;
                    }
                    seen_digit = true;
                    self.cursor += 1;
                }
                b'+' | b'-' => {
                    if seen_chomp {
                        valid = false;
                    }
                    seen_chomp = true;
                    self.cursor += 1;
                }
                _ => break,
            }
        }
        if !valid {
            self.problem(
                LexDiagnosticKind::InvalidBlockScalarHeader,
                start,
                self.cursor,
            );
        }
        self.push(SyntaxKind::BlockScalarHeader, start, self.cursor);
        if !valid {
            self.mark_last_error();
        }
    }

    fn scan_anchor(&mut self, kind: SyntaxKind, start: usize) {
        self.cursor = start + 1;
        let name_start = self.cursor;
        while self.cursor < self.source.len() {
            let byte = self.bytes()[self.cursor];
            if matches!(
                byte,
                b' ' | b'\t' | b'\r' | b'\n' | b',' | b'[' | b']' | b'{' | b'}'
            ) || is_forbidden_control(byte)
            {
                break;
            }
            self.cursor = next_boundary(self.source, self.cursor);
        }
        if self.cursor == name_start {
            self.problem(
                LexDiagnosticKind::InvalidAnchorName,
                start,
                self.cursor.max(start + 1),
            );
        }
        self.push(kind, start, self.cursor);
        if self.cursor == name_start {
            self.mark_last_error();
        }
    }

    fn scan_tag(&mut self, start: usize) {
        self.cursor = start + 1;
        if self.bytes().get(self.cursor) == Some(&b'<') {
            self.scan_verbatim_tag(start);
            return;
        }
        let run_start = self.cursor;
        while self.cursor < self.source.len() {
            let byte = self.bytes()[self.cursor];
            if matches!(
                byte,
                b' ' | b'\t' | b'\r' | b'\n' | b',' | b'[' | b']' | b'{' | b'}'
            ) || is_forbidden_control(byte)
            {
                break;
            }
            self.cursor = next_boundary(self.source, self.cursor);
        }
        // `!` and `!suffix` are valid; a `!` handle requires a non-empty
        // suffix after it, so `!!` and `!handle!` are malformed.
        let run = &self.source[run_start..self.cursor];
        let valid = run.find('!').is_none_or(|index| index + 1 < run.len());
        if !valid {
            self.problem(LexDiagnosticKind::InvalidTag, start, self.cursor);
        }
        self.push(SyntaxKind::Tag, start, self.cursor);
        if !valid {
            self.mark_last_error();
        }
    }

    fn scan_verbatim_tag(&mut self, start: usize) {
        let mut valid = true;
        self.cursor += 1;
        while self.cursor < self.source.len() {
            let byte = self.bytes()[self.cursor];
            match byte {
                b'>' => {
                    self.cursor += 1;
                    break;
                }
                _ if matches!(byte, b' ' | b'\t' | b'\r' | b'\n') || is_forbidden_control(byte) => {
                    valid = false;
                    self.cursor = next_boundary(self.source, self.cursor);
                }
                _ => self.cursor = next_boundary(self.source, self.cursor),
            }
        }
        if self.cursor >= self.source.len() && !self.source[..self.cursor].ends_with('>') {
            valid = false;
        }
        if !valid {
            self.problem(LexDiagnosticKind::InvalidTag, start, self.cursor);
        }
        self.push(SyntaxKind::Tag, start, self.cursor);
        if !valid {
            self.mark_last_error();
        }
    }

    fn scan_atom(&mut self, start: usize) {
        let byte = self.bytes()[start];
        if byte < 0x80 && is_forbidden_control(byte) {
            self.cursor = start + 1;
            self.push(SyntaxKind::Error, start, self.cursor);
            self.problem(LexDiagnosticKind::UnexpectedCharacter, start, self.cursor);
        } else {
            self.scan_plain(start);
        }
    }

    /// Scans a single-line plain scalar. The parser folds multi-line plain
    /// scalars by joining consecutive [`SyntaxKind::PlainScalar`] pieces across
    /// [`SyntaxKind::LineBreak`] tokens. Trailing spaces before a comment stay
    /// in the token so folding can trim them losslessly.
    fn scan_plain(&mut self, start: usize) {
        let mut previous_blank = false;
        self.cursor = start;
        while self.cursor < self.source.len() {
            let byte = self.bytes()[self.cursor];
            match byte {
                b'\r' | b'\n' => break,
                b'#' if previous_blank => break,
                b':' if self.plain_colon_terminates(self.cursor) => break,
                b',' | b'[' | b']' | b'{' | b'}' if self.flow_depth > 0 => break,
                b' ' | b'\t' => {
                    previous_blank = true;
                    self.cursor += 1;
                }
                byte if is_forbidden_control(byte) => break,
                _ => {
                    previous_blank = false;
                    self.cursor = next_boundary(self.source, self.cursor);
                }
            }
        }
        self.push(SyntaxKind::PlainScalar, start, self.cursor);
    }

    fn plain_colon_terminates(&self, offset: usize) -> bool {
        if self.blank_or_break_at(offset + 1) {
            return true;
        }
        self.flow_depth > 0
            && matches!(
                self.bytes().get(offset + 1),
                Some(b',' | b'[' | b']' | b'{' | b'}')
            )
    }

    fn scan_double_quoted(&mut self, start: usize) {
        let mut valid = true;
        self.cursor = start + 1;
        while self.cursor < self.source.len() {
            match self.bytes()[self.cursor] {
                b'"' => {
                    self.cursor += 1;
                    self.push(SyntaxKind::DoubleQuotedScalar, start, self.cursor);
                    if !valid {
                        self.mark_last_error();
                    }
                    return;
                }
                b'\\' => valid &= self.scan_double_quoted_escape(),
                b'\r' | b'\n' => self.consume_break(),
                byte if is_forbidden_control(byte) => {
                    let error_start = self.cursor;
                    self.cursor += 1;
                    valid = false;
                    self.problem(
                        LexDiagnosticKind::UnescapedControlCharacter,
                        error_start,
                        self.cursor,
                    );
                }
                _ => self.cursor = next_boundary(self.source, self.cursor),
            }
        }
        self.problem(
            LexDiagnosticKind::UnterminatedQuotedScalar,
            start,
            self.source.len(),
        );
        self.push(SyntaxKind::DoubleQuotedScalar, start, self.cursor);
        self.mark_last_error();
    }

    fn scan_double_quoted_escape(&mut self) -> bool {
        let start = self.cursor;
        self.cursor += 1;
        let Some(&escaped) = self.bytes().get(self.cursor) else {
            self.problem(LexDiagnosticKind::InvalidEscape, start, self.cursor);
            return false;
        };
        match escaped {
            b'0' | b'a' | b'b' | b't' | b'\t' | b'n' | b'v' | b'f' | b'r' | b'e' | b' ' | b'"'
            | b'/' | b'\\' | b'N' | b'_' | b'L' | b'P' => {
                self.cursor += 1;
                true
            }
            b'x' | b'u' | b'U' => {
                let width = match escaped {
                    b'x' => 2,
                    b'u' => 4,
                    _ => 8,
                };
                self.cursor += 1;
                let digits_start = self.cursor;
                let available_end = self.cursor.saturating_add(width).min(self.source.len());
                let valid = available_end - digits_start == width
                    && self.bytes()[digits_start..available_end]
                        .iter()
                        .all(u8::is_ascii_hexdigit);
                if valid {
                    self.cursor = available_end;
                } else {
                    self.problem(
                        LexDiagnosticKind::InvalidUnicodeEscape,
                        start,
                        self.cursor.max(start + 2),
                    );
                }
                valid
            }
            // An escaped line break continues the scalar without content.
            b'\r' | b'\n' => {
                self.consume_break();
                true
            }
            _ => {
                self.cursor = next_boundary(self.source, self.cursor);
                self.problem(LexDiagnosticKind::InvalidEscape, start, self.cursor);
                false
            }
        }
    }

    fn scan_single_quoted(&mut self, start: usize) {
        let mut valid = true;
        self.cursor = start + 1;
        while self.cursor < self.source.len() {
            match self.bytes()[self.cursor] {
                b'\'' => {
                    if self.bytes().get(self.cursor + 1) == Some(&b'\'') {
                        self.cursor += 2;
                    } else {
                        self.cursor += 1;
                        self.push(SyntaxKind::SingleQuotedScalar, start, self.cursor);
                        if !valid {
                            self.mark_last_error();
                        }
                        return;
                    }
                }
                b'\r' | b'\n' => self.consume_break(),
                byte if is_forbidden_control(byte) => {
                    let error_start = self.cursor;
                    self.cursor += 1;
                    valid = false;
                    self.problem(
                        LexDiagnosticKind::UnescapedControlCharacter,
                        error_start,
                        self.cursor,
                    );
                }
                _ => self.cursor = next_boundary(self.source, self.cursor),
            }
        }
        self.problem(
            LexDiagnosticKind::UnterminatedQuotedScalar,
            start,
            self.source.len(),
        );
        self.push(SyntaxKind::SingleQuotedScalar, start, self.cursor);
        self.mark_last_error();
    }

    fn matches_at(&self, offset: usize, prefix: &[u8]) -> bool {
        self.bytes()[offset..].starts_with(prefix)
    }

    fn blank_or_break_at(&self, offset: usize) -> bool {
        self.bytes()
            .get(offset)
            .is_none_or(|byte| is_blank_or_break(*byte))
    }

    fn push(&mut self, kind: SyntaxKind, start: usize, end: usize) {
        debug_assert!(start < end);
        debug_assert!(self.source.is_char_boundary(start));
        debug_assert!(self.source.is_char_boundary(end));
        if kind == SyntaxKind::LineBreak {
            self.line_start_offset = end;
            self.line_has_content = false;
        } else if !matches!(
            kind,
            SyntaxKind::Whitespace | SyntaxKind::Bom | SyntaxKind::Comment
        ) {
            self.line_has_content = true;
        }
        self.tokens.push(LexToken {
            kind,
            span: Span::new(start, end),
            flags: TokenFlags::EMPTY,
        });
    }

    fn mark_last_error(&mut self) {
        if let Some(last) = self.tokens.last_mut() {
            last.flags.0 |= TokenFlags::HAS_ERROR.0;
        }
    }

    fn problem(&mut self, kind: LexDiagnosticKind, start: usize, end: usize) {
        if self.diagnostics.len() < self.options.max_diagnostics {
            self.diagnostics.push(LexDiagnostic {
                kind,
                span: Span::new(start, end),
            });
        } else if !self.diagnostics_truncated {
            self.diagnostics_truncated = true;
            self.diagnostics.push(LexDiagnostic {
                kind: LexDiagnosticKind::TooManyDiagnostics,
                span: Span::new(start, start),
            });
        }
    }
}

const fn is_forbidden_control(byte: u8) -> bool {
    matches!(byte, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0x7F)
}

const fn is_blank_or_break(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn next_boundary(source: &str, cursor: usize) -> usize {
    let mut cursor = cursor + 1;
    while cursor < source.len() && !source.is_char_boundary(cursor) {
        cursor += 1;
    }
    cursor
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
        let rebuilt: String = lexed
            .tokens()
            .iter()
            .map(|token| token.text(source).unwrap_or_default())
            .collect();
        assert_eq!(rebuilt, source);
        let mut previous_end = 0;
        for token in lexed.tokens() {
            assert!(token.span.start >= previous_end, "{source:?}");
            assert!(token.span.end > token.span.start, "{source:?}");
            assert!(source.is_char_boundary(token.span.start), "{source:?}");
            assert!(source.is_char_boundary(token.span.end), "{source:?}");
            previous_end = token.span.end;
        }
        lexed
    }

    #[test]
    fn block_mapping_shape() {
        assert_eq!(
            kinds("key: value\nother: 42\n"),
            vec![
                SyntaxKind::PlainScalar,
                SyntaxKind::ValueIndicator,
                SyntaxKind::Whitespace,
                SyntaxKind::PlainScalar,
                SyntaxKind::LineBreak,
                SyntaxKind::PlainScalar,
                SyntaxKind::ValueIndicator,
                SyntaxKind::Whitespace,
                SyntaxKind::PlainScalar,
                SyntaxKind::LineBreak,
            ]
        );
        assert!(
            assert_lossless("key: value\nother: 42\n")
                .diagnostics()
                .is_empty()
        );
    }

    #[test]
    fn block_sequence_and_entries() {
        assert_eq!(
            kinds("- alpha\n  - nested\n- beta\n"),
            vec![
                SyntaxKind::BlockEntry,
                SyntaxKind::Whitespace,
                SyntaxKind::PlainScalar,
                SyntaxKind::LineBreak,
                SyntaxKind::Whitespace,
                SyntaxKind::BlockEntry,
                SyntaxKind::Whitespace,
                SyntaxKind::PlainScalar,
                SyntaxKind::LineBreak,
                SyntaxKind::BlockEntry,
                SyntaxKind::Whitespace,
                SyntaxKind::PlainScalar,
                SyntaxKind::LineBreak,
            ]
        );
    }

    #[test]
    fn flow_collections_shape() {
        assert_eq!(
            kinds("[1, {a: b}]\n"),
            vec![
                SyntaxKind::FlowSequenceStart,
                SyntaxKind::PlainScalar,
                SyntaxKind::FlowEntry,
                SyntaxKind::Whitespace,
                SyntaxKind::FlowMappingStart,
                SyntaxKind::PlainScalar,
                SyntaxKind::ValueIndicator,
                SyntaxKind::Whitespace,
                SyntaxKind::PlainScalar,
                SyntaxKind::FlowMappingEnd,
                SyntaxKind::FlowSequenceEnd,
                SyntaxKind::LineBreak,
            ]
        );
    }

    #[test]
    fn document_markers_and_directives() {
        assert_eq!(
            kinds("%YAML 1.2\n---\nfoo\n...\n"),
            vec![
                SyntaxKind::Directive,
                SyntaxKind::LineBreak,
                SyntaxKind::DocumentStart,
                SyntaxKind::LineBreak,
                SyntaxKind::PlainScalar,
                SyntaxKind::LineBreak,
                SyntaxKind::DocumentEnd,
                SyntaxKind::LineBreak,
            ]
        );
    }

    #[test]
    fn anchors_aliases_and_tags() {
        let source =
            "a: &anchor 1\nb: *anchor\nc: !!str v\nd: !tag v\ne: !<tag:example.com,2000:x> w\n";
        let lexed = assert_lossless(source);
        assert!(lexed.diagnostics().is_empty());
        let kinds: Vec<_> = lexed.tokens().iter().map(|token| token.kind).collect();
        assert!(kinds.contains(&SyntaxKind::Anchor));
        assert!(kinds.contains(&SyntaxKind::Alias));
        assert_eq!(
            kinds
                .iter()
                .filter(|kind| **kind == SyntaxKind::Tag)
                .count(),
            3
        );
    }

    #[test]
    fn quoted_scalars_span_lines_and_decode_later() {
        let source = "a: \"line one\n  line two\"\nb: 'it''s'\nc: \"\\u00e9\\t\"\n";
        let lexed = assert_lossless(source);
        assert!(lexed.diagnostics().is_empty());
        let double = lexed
            .tokens()
            .iter()
            .find(|token| token.kind == SyntaxKind::DoubleQuotedScalar)
            .unwrap();
        assert_eq!(double.text(source), Some("\"line one\n  line two\""));
    }

    #[test]
    fn block_scalar_headers_keep_indicators() {
        assert_eq!(
            kinds("a: |2-\n    kept\n"),
            vec![
                SyntaxKind::PlainScalar,
                SyntaxKind::ValueIndicator,
                SyntaxKind::Whitespace,
                SyntaxKind::BlockScalarHeader,
                SyntaxKind::LineBreak,
                SyntaxKind::Whitespace,
                SyntaxKind::PlainScalar,
                SyntaxKind::LineBreak,
            ]
        );
        assert!(
            assert_lossless(">\n  folded\n  text\n\n  more\n")
                .diagnostics()
                .is_empty()
        );
    }

    #[test]
    fn plain_scalars_keep_urls_and_colons() {
        let source = "url: http://example.com:8080/path#frag\nratio: a:b\n";
        let lexed = assert_lossless(source);
        assert!(lexed.diagnostics().is_empty());
        let plain: Vec<&str> = lexed
            .tokens()
            .iter()
            .filter(|token| token.kind == SyntaxKind::PlainScalar)
            .map(|token| token.text(source).unwrap())
            .collect();
        assert_eq!(
            plain,
            vec!["url", "http://example.com:8080/path#frag", "ratio", "a:b"]
        );
    }

    #[test]
    fn comments_and_tabs_as_separation() {
        let source = "key:\tvalue # trailing\n# whole line\n\n";
        let lexed = assert_lossless(source);
        assert!(lexed.diagnostics().is_empty());
    }

    #[test]
    fn bom_only_leads_the_stream() {
        assert_eq!(kinds("\u{feff}---\n")[0], SyntaxKind::Bom);
        assert!(assert_lossless("\u{feff}a: 1\n").diagnostics().is_empty());
    }

    #[test]
    fn tab_indentation_is_diagnosed_but_lossless() {
        let lexed = assert_lossless("\ta: 1\n  \tb: 2\n");
        assert_eq!(
            lexed
                .diagnostics()
                .iter()
                .map(|diagnostic| (diagnostic.kind, diagnostic.span))
                .collect::<Vec<_>>(),
            vec![
                (LexDiagnosticKind::TabInIndentation, Span::new(0, 1)),
                (LexDiagnosticKind::TabInIndentation, Span::new(6, 9)),
            ]
        );
    }

    #[test]
    fn unexpected_characters_include_c0() {
        let lexed = assert_lossless("a: b\u{1}\n");
        assert_eq!(lexed.diagnostics().len(), 1);
        assert_eq!(
            lexed.diagnostics()[0].kind,
            LexDiagnosticKind::UnexpectedCharacter
        );
        assert_eq!(lexed.diagnostics()[0].span, Span::new(4, 5));
    }

    #[test]
    fn unicode_printables_are_content() {
        // YAML 1.2 c-printable includes U+0085 (NEL), U+2028 (LS), U+2029 (PS)
        let lexed = assert_lossless("a: b\u{85}c\n");
        assert!(lexed.diagnostics().is_empty());
        let plain = lexed
            .tokens()
            .iter()
            .find(|token| token.kind == SyntaxKind::PlainScalar && token.span.start == 3)
            .unwrap();
        assert_eq!(plain.text("a: b\u{85}c\n"), Some("b\u{85}c"));
    }

    #[test]
    fn double_quoted_escape_diagnostics() {
        let lexed = assert_lossless("\"\\q\\u12\"");
        assert_eq!(
            lexed
                .diagnostics()
                .iter()
                .map(|diagnostic| (diagnostic.kind, diagnostic.span))
                .collect::<Vec<_>>(),
            vec![
                (LexDiagnosticKind::InvalidEscape, Span::new(1, 3)),
                (LexDiagnosticKind::InvalidUnicodeEscape, Span::new(3, 5)),
            ]
        );
    }

    #[test]
    fn double_quoted_supports_all_escape_widths() {
        for source in [
            "\"\\x41\"",
            "\"\\u0042\"",
            "\"\\U00000043\"",
            "\"\\\nnext\"",
        ] {
            let lexed = assert_lossless(source);
            assert!(lexed.diagnostics().is_empty(), "{source}");
        }
    }

    #[test]
    fn control_characters_in_scalars_and_comments() {
        let lexed = assert_lossless("\"a\u{7f}b\"\n# c\u{b}\n");
        assert_eq!(
            lexed
                .diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.kind)
                .collect::<Vec<_>>(),
            vec![
                LexDiagnosticKind::UnescapedControlCharacter,
                LexDiagnosticKind::UnescapedControlCharacter,
            ]
        );
        assert!(lexed.tokens().iter().any(|token| token.has_error()));
    }

    #[test]
    fn unterminated_quoted_scalars_recover_to_eof() {
        for source in ["\"abc", "'abc"] {
            let lexed = assert_lossless(source);
            assert_eq!(lexed.diagnostics().len(), 1);
            assert_eq!(
                lexed.diagnostics()[0].kind,
                LexDiagnosticKind::UnterminatedQuotedScalar
            );
            assert_eq!(lexed.diagnostics()[0].span, Span::new(0, source.len()));
            assert!(lexed.tokens()[0].has_error());
        }
    }

    #[test]
    fn invalid_anchors_tags_directives_and_block_headers() {
        let anchor = assert_lossless("a: & v\n");
        assert_eq!(
            anchor.diagnostics()[0].kind,
            LexDiagnosticKind::InvalidAnchorName
        );
        assert_eq!(anchor.diagnostics()[0].span, Span::new(3, 4));

        let tag = assert_lossless("a: !! v\n");
        assert_eq!(tag.diagnostics()[0].kind, LexDiagnosticKind::InvalidTag);
        assert_eq!(tag.diagnostics()[0].span, Span::new(3, 5));

        let bare_handle = assert_lossless("a: !e! v\n");
        assert_eq!(
            bare_handle.diagnostics()[0].kind,
            LexDiagnosticKind::InvalidTag
        );

        let directive = assert_lossless("%bad\u{1}\n");
        assert_eq!(
            directive.diagnostics()[0].kind,
            LexDiagnosticKind::InvalidDirective
        );

        for header in ["a: |0\n", "a: |22\n", "a: >+-\n"] {
            let lexed = assert_lossless(header);
            assert_eq!(
                lexed.diagnostics()[0].kind,
                LexDiagnosticKind::InvalidBlockScalarHeader,
                "{header}"
            );
        }
    }

    #[test]
    fn verbatim_tags_require_closing_bracket() {
        let closed = assert_lossless("a: !<tag:example.com,2000:x> v\n");
        assert!(closed.diagnostics().is_empty());

        let open = assert_lossless("a: !<tag:unterminated\n");
        assert_eq!(open.diagnostics()[0].kind, LexDiagnosticKind::InvalidTag);
    }

    #[test]
    fn colon_rules_differ_by_context() {
        assert_eq!(
            kinds("a:\n"),
            vec![
                SyntaxKind::PlainScalar,
                SyntaxKind::ValueIndicator,
                SyntaxKind::LineBreak,
            ]
        );
        assert_eq!(
            kinds("{a:b}\n")[1],
            SyntaxKind::PlainScalar,
            "no blank after the colon keeps it inside the scalar in flow"
        );
        let lexed = assert_lossless("{a: b}\n");
        assert!(lexed.diagnostics().is_empty());
    }

    #[test]
    fn crlf_and_lone_cr_are_line_breaks() {
        let lexed = assert_lossless("a: 1\r\nb: 2\rc: 3\n");
        assert!(lexed.diagnostics().is_empty());
        let breaks: Vec<Span> = lexed
            .tokens()
            .iter()
            .filter(|token| token.kind == SyntaxKind::LineBreak)
            .map(|token| token.span)
            .collect();
        assert_eq!(
            breaks,
            vec![Span::new(4, 6), Span::new(10, 11), Span::new(15, 16)]
        );
    }

    #[test]
    fn malformed_document_stays_lossless() {
        let source = "\ta: \"unterminated\n  &anchor: [1, {b: |0\n#c\u{2}";
        let lexed = assert_lossless(source);
        assert!(!lexed.diagnostics().is_empty());
        assert!(lexed.has_errors());
    }

    #[test]
    fn resource_limits_keep_output_lossless_and_bounded() {
        let source = "\t\n\t\n\t\n\t\n";
        let lexed = lex_with(source, LexerOptions::new().max_diagnostics(3));
        assert_eq!(lexed.diagnostics().len(), 4);
        assert_eq!(
            lexed.diagnostics().last().unwrap().kind,
            LexDiagnosticKind::TooManyDiagnostics
        );
        assert!(lexed.is_truncated());

        let limited = lex_with("[1,2,3,4,5,6]", LexerOptions::new().max_tokens(5));
        let rebuilt: String = limited
            .tokens()
            .iter()
            .map(|token| token.text("[1,2,3,4,5,6]").unwrap_or_default())
            .collect();
        assert_eq!(rebuilt, "[1,2,3,4,5,6]");
        assert!(limited.is_truncated());
        assert_eq!(limited.tokens().last().unwrap().kind, SyntaxKind::Error);

        let big = lex_with(
            "0123456789012345678901",
            LexerOptions::new().max_input_bytes(16),
        );
        assert_eq!(big.tokens().len(), 1);
        assert_eq!(big.tokens()[0].kind, SyntaxKind::Error);
        assert_eq!(
            big.diagnostics()[0].kind,
            LexDiagnosticKind::InputLimitExceeded
        );
        assert!(big.is_truncated());
    }

    #[test]
    fn diagnostic_endpoints_never_split_crlf() {
        let source = "\"a\u{7f}b\r\nc\u{b}\r\n";
        let lexed = lex(source);
        assert!(!lexed.diagnostics().is_empty());
        for diagnostic in lexed.diagnostics() {
            assert!(
                !source[..diagnostic.span.end].ends_with('\r'),
                "diagnostic end {end} splits a CRLF",
                end = diagnostic.span.end
            );
        }
    }

    #[test]
    fn significant_tokens_exclude_only_trivia() {
        let lexed = lex("key: value # note\n");
        let significant: Vec<SyntaxKind> =
            lexed.significant_tokens().map(|token| token.kind).collect();
        assert_eq!(
            significant,
            vec![
                SyntaxKind::PlainScalar,
                SyntaxKind::ValueIndicator,
                SyntaxKind::PlainScalar,
                SyntaxKind::LineBreak,
            ]
        );
    }
}
