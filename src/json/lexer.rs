//! A lossless, single-pass JSON and JSON-with-comments lexer.

use std::{error::Error, fmt};

use crate::Span;

/// Exact lexical categories emitted by the JSON lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxKind {
    Whitespace,
    Bom,
    LineComment,
    BlockComment,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Comma,
    Colon,
    String,
    Number,
    True,
    False,
    Null,
    Error,
}

impl SyntaxKind {
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::Whitespace | Self::Bom | Self::LineComment | Self::BlockComment
        )
    }

    #[must_use]
    pub const fn is_punctuation(self) -> bool {
        matches!(
            self,
            Self::LeftBrace
                | Self::RightBrace
                | Self::LeftBracket
                | Self::RightBracket
                | Self::Comma
                | Self::Colon
        )
    }

    #[must_use]
    pub const fn can_start_value(self) -> bool {
        matches!(
            self,
            Self::LeftBrace
                | Self::LeftBracket
                | Self::String
                | Self::Number
                | Self::True
                | Self::False
                | Self::Null
        )
    }
}

/// Compact per-token state that survives diagnostic truncation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TokenFlags(u8);

impl TokenFlags {
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

/// A specific JSON number grammar violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NumberIssue {
    MissingInteger,
    LeadingZero,
    MissingFractionDigits,
    MissingExponentDigits,
}

/// The category of a lexical diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LexDiagnosticKind {
    UnexpectedCharacter,
    UnexpectedToken,
    InvalidEscape,
    InvalidUnicodeEscape,
    UnescapedControlCharacter,
    UnterminatedString,
    UnterminatedBlockComment,
    InvalidNumber(NumberIssue),
    CommentsNotAllowed,
    BomNotAllowed,
    InputLimitExceeded,
    TokenLimitExceeded,
    TooManyDiagnostics,
}

impl LexDiagnosticKind {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnexpectedCharacter => "unexpected-character",
            Self::UnexpectedToken => "unexpected-token",
            Self::InvalidEscape => "invalid-escape",
            Self::InvalidUnicodeEscape => "invalid-unicode-escape",
            Self::UnescapedControlCharacter => "unescaped-control-character",
            Self::UnterminatedString => "unterminated-string",
            Self::UnterminatedBlockComment => "unterminated-block-comment",
            Self::InvalidNumber(NumberIssue::MissingInteger) => "number-missing-integer",
            Self::InvalidNumber(NumberIssue::LeadingZero) => "number-leading-zero",
            Self::InvalidNumber(NumberIssue::MissingFractionDigits) => {
                "number-missing-fraction-digits"
            }
            Self::InvalidNumber(NumberIssue::MissingExponentDigits) => {
                "number-missing-exponent-digits"
            }
            Self::CommentsNotAllowed => "comments-not-allowed",
            Self::BomNotAllowed => "bom-not-allowed",
            Self::InputLimitExceeded => "input-limit-exceeded",
            Self::TokenLimitExceeded => "token-limit-exceeded",
            Self::TooManyDiagnostics => "too-many-lex-diagnostics",
        }
    }
}

impl fmt::Display for LexDiagnosticKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnexpectedCharacter => "unexpected character in JSON",
            Self::UnexpectedToken => "unexpected token in JSON",
            Self::InvalidEscape => "invalid JSON string escape",
            Self::InvalidUnicodeEscape => "expected four hexadecimal digits after `\\u`",
            Self::UnescapedControlCharacter => {
                "JSON strings cannot contain unescaped control characters"
            }
            Self::UnterminatedString => "unterminated JSON string",
            Self::UnterminatedBlockComment => "unterminated block comment",
            Self::InvalidNumber(NumberIssue::MissingInteger) => {
                "expected an integer after the minus sign"
            }
            Self::InvalidNumber(NumberIssue::LeadingZero) => {
                "JSON numbers cannot contain a leading zero"
            }
            Self::InvalidNumber(NumberIssue::MissingFractionDigits) => {
                "expected at least one digit after the decimal point"
            }
            Self::InvalidNumber(NumberIssue::MissingExponentDigits) => {
                "expected at least one exponent digit"
            }
            Self::CommentsNotAllowed => "comments are not allowed in strict JSON",
            Self::BomNotAllowed => "a byte-order mark is not allowed in strict JSON",
            Self::InputLimitExceeded => "the configured JSON input byte limit was exceeded",
            Self::TokenLimitExceeded => "the configured JSON token limit was exceeded",
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

/// Lexer configuration. Constructors and builders keep future additions
/// backward-compatible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LexerOptions {
    allow_comments: bool,
    allow_bom: bool,
    max_input_bytes: usize,
    max_tokens: usize,
    max_diagnostics: usize,
}

impl LexerOptions {
    #[must_use]
    pub const fn strict() -> Self {
        Self {
            allow_comments: false,
            allow_bom: false,
            max_input_bytes: 16 * 1024 * 1024,
            max_tokens: 1_000_000,
            max_diagnostics: 256,
        }
    }

    #[must_use]
    pub const fn jsonc() -> Self {
        Self {
            allow_comments: true,
            allow_bom: true,
            max_input_bytes: 16 * 1024 * 1024,
            max_tokens: 1_000_000,
            max_diagnostics: 256,
        }
    }

    #[must_use]
    pub const fn allow_comments(mut self, yes: bool) -> Self {
        self.allow_comments = yes;
        self
    }

    #[must_use]
    pub const fn allow_bom(mut self, yes: bool) -> Self {
        self.allow_bom = yes;
        self
    }

    #[must_use]
    pub const fn max_tokens(mut self, max_tokens: usize) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    #[must_use]
    pub const fn max_input_bytes(mut self, max_input_bytes: usize) -> Self {
        self.max_input_bytes = max_input_bytes;
        self
    }

    #[must_use]
    pub const fn max_diagnostics(mut self, max_diagnostics: usize) -> Self {
        self.max_diagnostics = max_diagnostics;
        self
    }

    #[must_use]
    pub const fn comments_allowed(self) -> bool {
        self.allow_comments
    }

    #[must_use]
    pub const fn bom_allowed(self) -> bool {
        self.allow_bom
    }

    #[must_use]
    pub const fn token_limit(self) -> usize {
        self.max_tokens
    }

    #[must_use]
    pub const fn input_byte_limit(self) -> usize {
        self.max_input_bytes
    }

    #[must_use]
    pub const fn diagnostic_limit(self) -> usize {
        self.max_diagnostics
    }
}

impl Default for LexerOptions {
    fn default() -> Self {
        Self::strict()
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

/// Lexes strict JSON.
#[must_use]
pub fn lex(source: &str) -> Lexed<'_> {
    lex_with(source, LexerOptions::strict())
}

/// Lexes JSON using the supplied strict/JSONC options.
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
            let byte = self.bytes()[start];
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' => self.scan_whitespace(start),
                b'{' => self.single(SyntaxKind::LeftBrace),
                b'}' => self.single(SyntaxKind::RightBrace),
                b'[' => self.single(SyntaxKind::LeftBracket),
                b']' => self.single(SyntaxKind::RightBracket),
                b',' => self.single(SyntaxKind::Comma),
                b':' => self.single(SyntaxKind::Colon),
                b'"' => self.scan_string(start),
                b'/' if self.bytes().get(start + 1) == Some(&b'/') => self.scan_line_comment(start),
                b'/' if self.bytes().get(start + 1) == Some(&b'*') => {
                    self.scan_block_comment(start)
                }
                b'-' | b'0'..=b'9' => self.scan_number(start),
                b't' | b'f' | b'n' | b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.scan_word(start),
                _ if start == 0 && self.source[start..].starts_with('\u{feff}') => {
                    self.cursor += '\u{feff}'.len_utf8();
                    self.push(SyntaxKind::Bom, start, self.cursor);
                    if !self.options.allow_bom {
                        self.problem(LexDiagnosticKind::BomNotAllowed, start, self.cursor);
                    }
                }
                _ => self.scan_unexpected(start),
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

    fn scan_whitespace(&mut self, start: usize) {
        self.cursor += 1;
        while self
            .bytes()
            .get(self.cursor)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.cursor += 1;
        }
        self.push(SyntaxKind::Whitespace, start, self.cursor);
    }

    fn scan_string(&mut self, start: usize) {
        let mut valid = true;
        self.cursor += 1;
        while self.cursor < self.source.len() {
            match self.bytes()[self.cursor] {
                b'"' => {
                    self.cursor += 1;
                    self.push(SyntaxKind::String, start, self.cursor);
                    if !valid {
                        self.mark_last_error();
                    }
                    return;
                }
                b'\\' => valid &= self.scan_escape(),
                b'\r' | b'\n' => {
                    let control_end = line_break_end(self.bytes(), self.cursor);
                    self.problem(
                        LexDiagnosticKind::UnescapedControlCharacter,
                        self.cursor,
                        control_end,
                    );
                    self.problem(LexDiagnosticKind::UnterminatedString, start, self.cursor);
                    self.push(SyntaxKind::String, start, self.cursor);
                    self.mark_last_error();
                    return;
                }
                0x00..=0x1f => {
                    valid = false;
                    let error_start = self.cursor;
                    self.cursor += 1;
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
            LexDiagnosticKind::UnterminatedString,
            start,
            self.source.len(),
        );
        self.push(SyntaxKind::String, start, self.cursor);
        self.mark_last_error();
    }

    fn scan_escape(&mut self) -> bool {
        let start = self.cursor;
        self.cursor += 1;
        let Some(&escaped) = self.bytes().get(self.cursor) else {
            self.problem(LexDiagnosticKind::InvalidEscape, start, self.cursor);
            return false;
        };
        match escaped {
            b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {
                self.cursor += 1;
                true
            }
            b'u' => {
                self.cursor += 1;
                let digits_start = self.cursor;
                let available_end = self.cursor.saturating_add(4).min(self.source.len());
                let valid = available_end - digits_start == 4
                    && self.bytes()[digits_start..available_end]
                        .iter()
                        .all(u8::is_ascii_hexdigit);
                if valid {
                    self.cursor = available_end;
                    true
                } else {
                    while self.cursor < available_end
                        && self.bytes()[self.cursor].is_ascii_alphanumeric()
                    {
                        self.cursor += 1;
                    }
                    self.problem(
                        LexDiagnosticKind::InvalidUnicodeEscape,
                        start,
                        self.cursor.max(start + 2),
                    );
                    false
                }
            }
            b'\r' | b'\n' => {
                self.problem(
                    LexDiagnosticKind::InvalidEscape,
                    start,
                    line_break_end(self.bytes(), self.cursor),
                );
                false
            }
            _ => {
                self.cursor = next_boundary(self.source, self.cursor);
                self.problem(LexDiagnosticKind::InvalidEscape, start, self.cursor);
                false
            }
        }
    }

    fn scan_line_comment(&mut self, start: usize) {
        self.cursor += 2;
        while self.cursor < self.source.len() && !matches!(self.bytes()[self.cursor], b'\r' | b'\n')
        {
            self.cursor = next_boundary(self.source, self.cursor);
        }
        self.push(SyntaxKind::LineComment, start, self.cursor);
        if !self.options.allow_comments {
            self.problem(LexDiagnosticKind::CommentsNotAllowed, start, self.cursor);
        }
    }

    fn scan_block_comment(&mut self, start: usize) {
        self.cursor += 2;
        while self.cursor < self.source.len() {
            if self.bytes().get(self.cursor) == Some(&b'*')
                && self.bytes().get(self.cursor + 1) == Some(&b'/')
            {
                self.cursor += 2;
                self.push(SyntaxKind::BlockComment, start, self.cursor);
                if !self.options.allow_comments {
                    self.problem(LexDiagnosticKind::CommentsNotAllowed, start, self.cursor);
                }
                return;
            }
            self.cursor = next_boundary(self.source, self.cursor);
        }
        self.push(SyntaxKind::BlockComment, start, self.cursor);
        if !self.options.allow_comments {
            self.problem(LexDiagnosticKind::CommentsNotAllowed, start, self.cursor);
        }
        self.problem(
            LexDiagnosticKind::UnterminatedBlockComment,
            start,
            self.cursor,
        );
    }

    fn scan_number(&mut self, start: usize) {
        self.cursor = start;
        if self.bytes().get(self.cursor) == Some(&b'-') {
            self.cursor += 1;
        }
        while self
            .bytes()
            .get(self.cursor)
            .is_some_and(u8::is_ascii_digit)
        {
            self.cursor += 1;
        }
        if self.bytes().get(self.cursor) == Some(&b'.') {
            self.cursor += 1;
            while self
                .bytes()
                .get(self.cursor)
                .is_some_and(u8::is_ascii_digit)
            {
                self.cursor += 1;
            }
        }
        if matches!(self.bytes().get(self.cursor), Some(b'e' | b'E')) {
            self.cursor += 1;
            if matches!(self.bytes().get(self.cursor), Some(b'+' | b'-')) {
                self.cursor += 1;
            }
            while self
                .bytes()
                .get(self.cursor)
                .is_some_and(u8::is_ascii_digit)
            {
                self.cursor += 1;
            }
        }
        let end = self.cursor;
        self.push(SyntaxKind::Number, start, end);
        if let Err((issue, relative_start, relative_end)) =
            validate_number(&self.source[start..end])
        {
            self.mark_last_error();
            self.problem(
                LexDiagnosticKind::InvalidNumber(issue),
                start + relative_start,
                start + relative_end,
            );
        }
    }

    fn scan_word(&mut self, start: usize) {
        self.cursor += 1;
        while self
            .bytes()
            .get(self.cursor)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            self.cursor += 1;
        }
        let text = &self.source[start..self.cursor];
        let kind = match text {
            "true" => SyntaxKind::True,
            "false" => SyntaxKind::False,
            "null" => SyntaxKind::Null,
            _ => SyntaxKind::Error,
        };
        self.push(kind, start, self.cursor);
        if kind == SyntaxKind::Error {
            self.problem(LexDiagnosticKind::UnexpectedToken, start, self.cursor);
        }
    }

    fn scan_unexpected(&mut self, start: usize) {
        self.cursor = next_boundary(self.source, self.cursor);
        while self.cursor < self.source.len() && !is_token_start(self.source, self.cursor) {
            self.cursor = next_boundary(self.source, self.cursor);
        }
        self.push(SyntaxKind::Error, start, self.cursor);
        self.problem(LexDiagnosticKind::UnexpectedCharacter, start, self.cursor);
    }

    fn push(&mut self, kind: SyntaxKind, start: usize, end: usize) {
        debug_assert!(start < end);
        debug_assert!(self.source.is_char_boundary(start));
        debug_assert!(self.source.is_char_boundary(end));
        self.tokens.push(LexToken {
            kind,
            span: Span::new(start, end),
            flags: TokenFlags::default(),
        });
    }

    fn mark_last_error(&mut self) {
        if let Some(token) = self.tokens.last_mut() {
            token.flags = TokenFlags::HAS_ERROR;
        }
    }

    fn problem(&mut self, kind: LexDiagnosticKind, start: usize, end: usize) {
        if self.diagnostics.len() < self.options.max_diagnostics {
            self.diagnostics.push(LexDiagnostic {
                kind,
                span: Span::new(start, end),
            });
        } else if !self.diagnostics_truncated {
            self.diagnostics.push(LexDiagnostic {
                kind: LexDiagnosticKind::TooManyDiagnostics,
                span: Span::new(start, start),
            });
            self.diagnostics_truncated = true;
        }
    }
}

fn is_token_start(source: &str, cursor: usize) -> bool {
    let byte = source.as_bytes()[cursor];
    matches!(
        byte,
        b' ' | b'\t'
            | b'\r'
            | b'\n'
            | b'{'
            | b'}'
            | b'['
            | b']'
            | b','
            | b':'
            | b'"'
            | b'/'
            | b'-'
            | b'0'..=b'9'
            | b'a'..=b'z'
            | b'A'..=b'Z'
            | b'_'
    ) || source[cursor..].starts_with('\u{feff}')
}

fn next_boundary(source: &str, start: usize) -> usize {
    start + source[start..].chars().next().map_or(1, char::len_utf8)
}

fn line_break_end(bytes: &[u8], cursor: usize) -> usize {
    if bytes.get(cursor) == Some(&b'\r') && bytes.get(cursor + 1) == Some(&b'\n') {
        cursor + 2
    } else {
        cursor + 1
    }
}

fn validate_number(text: &str) -> Result<(), (NumberIssue, usize, usize)> {
    let bytes = text.as_bytes();
    let mut cursor = 0;
    if bytes.get(cursor) == Some(&b'-') {
        cursor += 1;
    }
    match bytes.get(cursor) {
        Some(b'0') => {
            cursor += 1;
            if bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
                return Err((NumberIssue::LeadingZero, cursor, cursor + 1));
            }
        }
        Some(b'1'..=b'9') => {
            cursor += 1;
            while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
                cursor += 1;
            }
        }
        _ => return Err((NumberIssue::MissingInteger, 0, text.len().max(1))),
    }

    if bytes.get(cursor) == Some(&b'.') {
        let decimal_point = cursor;
        cursor += 1;
        let digits = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == digits {
            return Err((
                NumberIssue::MissingFractionDigits,
                decimal_point,
                decimal_point + 1,
            ));
        }
    }

    if matches!(bytes.get(cursor), Some(b'e' | b'E')) {
        let exponent = cursor;
        cursor += 1;
        if matches!(bytes.get(cursor), Some(b'+' | b'-')) {
            cursor += 1;
        }
        let digits = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == digits {
            return Err((NumberIssue::MissingExponentDigits, exponent, cursor));
        }
    }

    debug_assert_eq!(cursor, bytes.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<SyntaxKind> {
        lex(source).tokens.iter().map(|token| token.kind).collect()
    }

    fn assert_lossless(source: &str, lexed: &Lexed<'_>) {
        let rebuilt: String = lexed
            .tokens()
            .iter()
            .map(|token| token.text(source).unwrap())
            .collect();
        assert_eq!(rebuilt, source);
        for pair in lexed.tokens().windows(2) {
            assert_eq!(pair[0].span.end, pair[1].span.start);
        }
        assert!(lexed.tokens().iter().all(|token| {
            token.span.start < token.span.end
                && source.is_char_boundary(token.span.start)
                && source.is_char_boundary(token.span.end)
        }));
    }

    #[test]
    fn emits_exact_json_categories() {
        let source = "{\"x\":[true,false,null,-1.2e+3]}";
        assert_eq!(
            kinds(source),
            vec![
                SyntaxKind::LeftBrace,
                SyntaxKind::String,
                SyntaxKind::Colon,
                SyntaxKind::LeftBracket,
                SyntaxKind::True,
                SyntaxKind::Comma,
                SyntaxKind::False,
                SyntaxKind::Comma,
                SyntaxKind::Null,
                SyntaxKind::Comma,
                SyntaxKind::Number,
                SyntaxKind::RightBracket,
                SyntaxKind::RightBrace,
            ]
        );
        assert!(lex(source).diagnostics().is_empty());
    }

    #[test]
    fn tokens_losslessly_cover_unicode_and_malformed_input() {
        for source in [
            "",
            "Москва 😀",
            "[\"unterminated\nnext]",
            "{\"x\":01,wat:@}",
            "\u{feff}// hi\r\n[1]",
        ] {
            assert_lossless(source, &lex(source));
        }
    }

    #[test]
    fn accepts_exact_number_grammar() {
        for source in ["0", "-0", "42", "-12.5", "1e9", "1E-9", "0.0e+0"] {
            assert!(lex(source).diagnostics().is_empty(), "{source}");
        }
        for (source, issue) in [
            ("-", NumberIssue::MissingInteger),
            ("01", NumberIssue::LeadingZero),
            ("1.", NumberIssue::MissingFractionDigits),
            ("1e+", NumberIssue::MissingExponentDigits),
        ] {
            assert_eq!(
                lex(source).diagnostics()[0].kind,
                LexDiagnosticKind::InvalidNumber(issue),
                "{source}"
            );
        }
        assert!(lex("1x").has_errors());
    }

    #[test]
    fn validates_string_escapes_and_recovers_at_newline() {
        assert!(
            lex(r#""\\\"\/\b\f\n\r\t\uD83D\uDE00""#)
                .diagnostics()
                .is_empty()
        );
        assert_eq!(
            lex(r#""\q""#).diagnostics()[0].kind,
            LexDiagnosticKind::InvalidEscape
        );
        assert_eq!(
            lex(r#""\u12xz""#).diagnostics()[0].kind,
            LexDiagnosticKind::InvalidUnicodeEscape
        );
        let recovered = lex("\"bad\ntrue");
        assert!(
            recovered
                .tokens()
                .iter()
                .any(|token| token.kind == SyntaxKind::True)
        );
    }

    #[test]
    fn comments_and_bom_are_mode_dependent_but_always_lossless() {
        let source = "\u{feff}/* block */ // line\nnull";
        assert!(lex(source).diagnostics().len() >= 3);
        let jsonc = lex_with(source, LexerOptions::jsonc());
        assert!(jsonc.diagnostics().is_empty());
        assert_lossless(source, &jsonc);
    }

    #[test]
    fn reports_unterminated_block_comment() {
        let result = lex_with("/* never ends", LexerOptions::jsonc());
        assert_eq!(
            result.diagnostics()[0].kind,
            LexDiagnosticKind::UnterminatedBlockComment
        );
    }

    #[test]
    fn malformed_fragments_do_not_swallow_following_values() {
        assert_eq!(
            kinds("1+2"),
            vec![SyntaxKind::Number, SyntaxKind::Error, SyntaxKind::Number]
        );
        assert_eq!(kinds("@true"), vec![SyntaxKind::Error, SyntaxKind::True]);
        assert_eq!(kinds("true-1"), vec![SyntaxKind::True, SyntaxKind::Number]);
        assert_eq!(
            kinds("1é2"),
            vec![SyntaxKind::Number, SyntaxKind::Error, SyntaxKind::Number]
        );
    }

    #[test]
    fn resource_limits_keep_output_lossless_and_bounded() {
        let noisy = format!("\"{}\"", "\u{1}".repeat(100));
        let diagnostics = lex_with(&noisy, LexerOptions::strict().max_diagnostics(3));
        assert_eq!(diagnostics.diagnostics().len(), 4);
        assert_eq!(
            diagnostics.diagnostics().last().unwrap().kind,
            LexDiagnosticKind::TooManyDiagnostics
        );

        let source = "[1,2,3,4]";
        let limited = lex_with(source, LexerOptions::strict().max_tokens(3));
        assert!(limited.is_truncated());
        assert!(limited.tokens().len() <= 4);
        assert_lossless(source, &limited);
    }

    #[test]
    fn diagnostic_endpoints_never_split_crlf() {
        let source = "\"x\r\n";
        let result = lex(source);
        let index = crate::LineIndex::new(source);
        for diagnostic in result.diagnostics() {
            assert!(
                index
                    .line_column(diagnostic.span.start, crate::ColumnEncoding::Utf8Bytes)
                    .is_ok()
            );
            assert!(
                index
                    .line_column(diagnostic.span.end, crate::ColumnEncoding::Utf8Bytes)
                    .is_ok()
            );
        }
    }

    #[test]
    fn malformed_number_diagnostics_anchor_the_failed_transition() {
        for (source, span) in [
            ("1.", Span::new(1, 2)),
            ("1.e2", Span::new(1, 2)),
            ("1e", Span::new(1, 2)),
            ("1e+", Span::new(1, 3)),
        ] {
            assert_eq!(lex(source).diagnostics()[0].span, span, "{source}");
        }
    }

    #[test]
    fn input_limit_short_circuits_as_one_lossless_error_token() {
        let source = "{\"large\":true}";
        let limited = lex_with(source, LexerOptions::strict().max_input_bytes(4));
        assert!(limited.is_truncated());
        assert_eq!(limited.tokens().len(), 1);
        assert_eq!(limited.tokens()[0].text(source), Some(source));
        assert_eq!(
            limited.diagnostics()[0].kind,
            LexDiagnosticKind::InputLimitExceeded
        );
    }
}
