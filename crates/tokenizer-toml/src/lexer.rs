//! A lossless, single-pass TOML 1.0 lexer.
//!
//! Token text concatenation always reconstructs the source byte-for-byte,
//! including malformed and incomplete editor text. Newlines are significant
//! tokens because TOML uses them to separate expressions.

use std::{error::Error, fmt};

use crate::Span;

/// Exact lexical categories emitted by the TOML lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxKind {
    Whitespace,
    Newline,
    Bom,
    LineComment,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Equals,
    Comma,
    Dot,
    BasicString,
    LiteralString,
    MultiLineBasicString,
    MultiLineLiteralString,
    BareKey,
    Integer,
    Float,
    HexInteger,
    OctInteger,
    BinInteger,
    True,
    False,
    Inf,
    Nan,
    OffsetDateTime,
    LocalDateTime,
    LocalDate,
    LocalTime,
    Error,
}

impl SyntaxKind {
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(self, Self::Whitespace | Self::Bom | Self::LineComment)
    }

    #[must_use]
    pub const fn is_punctuation(self) -> bool {
        matches!(
            self,
            Self::LeftBracket
                | Self::RightBracket
                | Self::LeftBrace
                | Self::RightBrace
                | Self::Equals
                | Self::Comma
                | Self::Dot
        )
    }

    #[must_use]
    pub const fn can_start_value(self) -> bool {
        matches!(
            self,
            Self::BasicString
                | Self::LiteralString
                | Self::MultiLineBasicString
                | Self::MultiLineLiteralString
                | Self::Integer
                | Self::Float
                | Self::HexInteger
                | Self::OctInteger
                | Self::BinInteger
                | Self::True
                | Self::False
                | Self::Inf
                | Self::Nan
                | Self::OffsetDateTime
                | Self::LocalDateTime
                | Self::LocalDate
                | Self::LocalTime
        )
    }

    /// Number, boolean, and date-time tokens may be reinterpreted as bare-key
    /// segments when their spelling fits the bare-key charset.
    #[must_use]
    pub const fn can_start_key(self) -> bool {
        matches!(
            self,
            Self::BareKey
                | Self::BasicString
                | Self::LiteralString
                | Self::Integer
                | Self::Float
                | Self::HexInteger
                | Self::OctInteger
                | Self::BinInteger
                | Self::True
                | Self::False
                | Self::Inf
                | Self::Nan
                | Self::OffsetDateTime
                | Self::LocalDateTime
                | Self::LocalDate
                | Self::LocalTime
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

/// A specific TOML number grammar violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NumberIssue {
    MissingDigits,
    LeadingZero,
    MisplacedUnderscore,
    MissingExponentDigits,
    InvalidRadixDigit,
    SignedRadix,
}

/// A specific TOML date-time grammar violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DateTimeIssue {
    MissingComponent,
    InvalidComponent,
}

/// The category of a lexical diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LexDiagnosticKind {
    UnexpectedCharacter,
    InvalidEscape,
    InvalidUnicodeEscape,
    UnescapedControlCharacter,
    UnterminatedString,
    InvalidNumber(NumberIssue),
    InvalidDateTime(DateTimeIssue),
    InvalidLineBreak,
    InputLimitExceeded,
    TokenLimitExceeded,
    TooManyDiagnostics,
}

impl LexDiagnosticKind {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnexpectedCharacter => "unexpected-character",
            Self::InvalidEscape => "invalid-escape",
            Self::InvalidUnicodeEscape => "invalid-unicode-escape",
            Self::UnescapedControlCharacter => "unescaped-control-character",
            Self::UnterminatedString => "unterminated-string",
            Self::InvalidNumber(NumberIssue::MissingDigits) => "number-missing-digits",
            Self::InvalidNumber(NumberIssue::LeadingZero) => "number-leading-zero",
            Self::InvalidNumber(NumberIssue::MisplacedUnderscore) => "number-misplaced-underscore",
            Self::InvalidNumber(NumberIssue::MissingExponentDigits) => {
                "number-missing-exponent-digits"
            }
            Self::InvalidNumber(NumberIssue::InvalidRadixDigit) => "number-invalid-radix-digit",
            Self::InvalidNumber(NumberIssue::SignedRadix) => "number-signed-radix",
            Self::InvalidDateTime(DateTimeIssue::MissingComponent) => "datetime-missing-component",
            Self::InvalidDateTime(DateTimeIssue::InvalidComponent) => "datetime-invalid-component",
            Self::InvalidLineBreak => "invalid-line-break",
            Self::InputLimitExceeded => "input-limit-exceeded",
            Self::TokenLimitExceeded => "token-limit-exceeded",
            Self::TooManyDiagnostics => "too-many-lex-diagnostics",
        }
    }
}

impl fmt::Display for LexDiagnosticKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnexpectedCharacter => "unexpected character in TOML",
            Self::InvalidEscape => "invalid TOML string escape",
            Self::InvalidUnicodeEscape => {
                "expected four hexadecimal digits after `\\u` or eight after `\\U`"
            }
            Self::UnescapedControlCharacter => {
                "TOML strings and comments cannot contain control characters other than tab"
            }
            Self::UnterminatedString => "unterminated TOML string",
            Self::InvalidNumber(NumberIssue::MissingDigits) => {
                "expected at least one digit in the TOML number"
            }
            Self::InvalidNumber(NumberIssue::LeadingZero) => {
                "TOML integers cannot contain a leading zero"
            }
            Self::InvalidNumber(NumberIssue::MisplacedUnderscore) => {
                "TOML underscores must be surrounded by digits"
            }
            Self::InvalidNumber(NumberIssue::MissingExponentDigits) => {
                "expected at least one exponent digit"
            }
            Self::InvalidNumber(NumberIssue::InvalidRadixDigit) => {
                "the digit is not valid for this radix prefix"
            }
            Self::InvalidNumber(NumberIssue::SignedRadix) => {
                "a sign is not allowed before a `0x`, `0o` or `0b` number"
            }
            Self::InvalidDateTime(DateTimeIssue::MissingComponent) => {
                "the TOML date-time is missing a required component"
            }
            Self::InvalidDateTime(DateTimeIssue::InvalidComponent) => {
                "the TOML date-time component is outside its valid range"
            }
            Self::InvalidLineBreak => "a carriage return must be followed by a line feed",
            Self::InputLimitExceeded => "the configured TOML input byte limit was exceeded",
            Self::TokenLimitExceeded => "the configured TOML token limit was exceeded",
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

/// Lexes TOML.
#[must_use]
pub fn lex(source: &str) -> Lexed<'_> {
    lex_with(source, LexerOptions::new())
}

/// Lexes TOML using the supplied options.
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
        mark_after_push: false,
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
    mark_after_push: bool,
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
                b'\n' => self.scan_newline(start),
                b'\r' => self.scan_carriage_return(start),
                b'#' => self.scan_comment(start),
                b'[' => self.single(SyntaxKind::LeftBracket),
                b']' => self.single(SyntaxKind::RightBracket),
                b'{' => self.single(SyntaxKind::LeftBrace),
                b'}' => self.single(SyntaxKind::RightBrace),
                b'=' => self.single(SyntaxKind::Equals),
                b',' => self.single(SyntaxKind::Comma),
                b'.' => self.single(SyntaxKind::Dot),
                b'"' => {
                    if self.bytes().get(start + 1) == Some(&b'"')
                        && self.bytes().get(start + 2) == Some(&b'"')
                    {
                        self.scan_multi_line_basic_string(start);
                    } else {
                        self.scan_basic_string(start);
                    }
                }
                b'\'' => {
                    if self.bytes().get(start + 1) == Some(&b'\'')
                        && self.bytes().get(start + 2) == Some(&b'\'')
                    {
                        self.scan_multi_line_literal_string(start);
                    } else {
                        self.scan_literal_string(start);
                    }
                }
                b'+' | b'-' => self.scan_signed(start),
                b'0'..=b'9' => self.scan_digit_atom(start),
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.scan_bare_atom(start),
                _ if start == 0 && self.source[start..].starts_with('\u{feff}') => {
                    self.cursor += '\u{feff}'.len_utf8();
                    self.push(SyntaxKind::Bom, start, self.cursor);
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
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            self.cursor += 1;
        }
        self.push(SyntaxKind::Whitespace, start, self.cursor);
    }

    fn scan_newline(&mut self, start: usize) {
        self.cursor += 1;
        self.push(SyntaxKind::Newline, start, self.cursor);
    }

    fn scan_carriage_return(&mut self, start: usize) {
        self.cursor += 1;
        if self.bytes().get(self.cursor) == Some(&b'\n') {
            self.cursor += 1;
            self.push(SyntaxKind::Newline, start, self.cursor);
        } else {
            self.push(SyntaxKind::Error, start, self.cursor);
            self.problem(LexDiagnosticKind::InvalidLineBreak, start, self.cursor);
        }
    }

    fn scan_comment(&mut self, start: usize) {
        self.cursor += 1;
        let mut valid = true;
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
        self.push(SyntaxKind::LineComment, start, self.cursor);
        if !valid {
            self.mark_last_error();
        }
    }

    fn scan_basic_string(&mut self, start: usize) {
        let mut valid = true;
        self.cursor += 1;
        while self.cursor < self.source.len() {
            match self.bytes()[self.cursor] {
                b'"' => {
                    self.cursor += 1;
                    self.push(SyntaxKind::BasicString, start, self.cursor);
                    if !valid {
                        self.mark_last_error();
                    }
                    return;
                }
                b'\\' => valid &= self.scan_escape(),
                b'\r' | b'\n' => {
                    let control_end = line_break_end(self.bytes(), self.cursor);
                    self.problem(
                        LexDiagnosticKind::UnterminatedString,
                        start,
                        control_end.max(start + 1),
                    );
                    self.cursor = control_end;
                    self.push(SyntaxKind::BasicString, start, self.cursor);
                    self.mark_last_error();
                    return;
                }
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
            LexDiagnosticKind::UnterminatedString,
            start,
            self.source.len(),
        );
        self.push(SyntaxKind::BasicString, start, self.cursor);
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
            b'"' | b'\\' | b'b' | b'f' | b'n' | b'r' | b't' => {
                self.cursor += 1;
                true
            }
            b'u' | b'U' => {
                let width = if escaped == b'u' { 4 } else { 8 };
                self.cursor += 1;
                let digits_start = self.cursor;
                let available_end = self.cursor.saturating_add(width).min(self.source.len());
                let valid = available_end - digits_start == width
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

    fn scan_multi_line_basic_string(&mut self, start: usize) {
        let mut valid = true;
        self.cursor = start + 3;
        if self.bytes().get(self.cursor) == Some(&b'\r') && self.bytes().get(self.cursor + 1) == Some(&b'\n') {
            self.cursor += 2;
        } else if self.bytes().get(self.cursor) == Some(&b'\n') {
            self.cursor += 1;
        }
        while self.cursor < self.source.len() {
            match self.bytes()[self.cursor] {
                b'"' => {
                    let run = self.quote_run(b'"');
                    if run >= 3 {
                        // The final three quotes close the token; any extra
                        // leading quotes belong to the string content.
                        self.cursor += run;
                        self.push(SyntaxKind::MultiLineBasicString, start, self.cursor);
                        if !valid {
                            self.mark_last_error();
                        }
                        return;
                    }
                    self.cursor += run;
                }
                b'\\' => valid &= self.scan_multi_line_escape(),
                b'\r' if self.bytes().get(self.cursor + 1) != Some(&b'\n') => {
                    let error_start = self.cursor;
                    self.cursor += 1;
                    valid = false;
                    self.problem(LexDiagnosticKind::InvalidLineBreak, error_start, self.cursor);
                }
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
            LexDiagnosticKind::UnterminatedString,
            start,
            self.source.len(),
        );
        self.push(SyntaxKind::MultiLineBasicString, start, self.cursor);
        self.mark_last_error();
    }

    fn scan_multi_line_escape(&mut self) -> bool {
        let escape_start = self.cursor;
        self.cursor += 1;
        let mut after = self.cursor;
        while self.bytes().get(after).is_some_and(|byte| matches!(byte, b' ' | b'\t')) {
            after += 1;
        }
        if self.bytes().get(after) == Some(&b'\n')
            || (self.bytes().get(after) == Some(&b'\r') && self.bytes().get(after + 1) == Some(&b'\n'))
        {
            self.cursor = after;
            while self
                .bytes()
                .get(self.cursor)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
            {
                self.cursor += 1;
            }
            return true;
        }
        self.cursor = escape_start;
        self.scan_escape()
    }

    fn scan_literal_string(&mut self, start: usize) {
        let mut valid = true;
        self.cursor += 1;
        while self.cursor < self.source.len() {
            match self.bytes()[self.cursor] {
                b'\'' => {
                    self.cursor += 1;
                    self.push(SyntaxKind::LiteralString, start, self.cursor);
                    if !valid {
                        self.mark_last_error();
                    }
                    return;
                }
                b'\r' | b'\n' => {
                    let control_end = line_break_end(self.bytes(), self.cursor);
                    self.problem(
                        LexDiagnosticKind::UnterminatedString,
                        start,
                        control_end.max(start + 1),
                    );
                    self.cursor = control_end;
                    self.push(SyntaxKind::LiteralString, start, self.cursor);
                    self.mark_last_error();
                    return;
                }
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
            LexDiagnosticKind::UnterminatedString,
            start,
            self.source.len(),
        );
        self.push(SyntaxKind::LiteralString, start, self.cursor);
        self.mark_last_error();
    }

    fn scan_multi_line_literal_string(&mut self, start: usize) {
        let mut valid = true;
        self.cursor = start + 3;
        if self.bytes().get(self.cursor) == Some(&b'\r') && self.bytes().get(self.cursor + 1) == Some(&b'\n') {
            self.cursor += 2;
        } else if self.bytes().get(self.cursor) == Some(&b'\n') {
            self.cursor += 1;
        }
        while self.cursor < self.source.len() {
            match self.bytes()[self.cursor] {
                b'\'' => {
                    let run = self.quote_run(b'\'');
                    if run >= 3 {
                        self.cursor += run;
                        self.push(SyntaxKind::MultiLineLiteralString, start, self.cursor);
                        if !valid {
                            self.mark_last_error();
                        }
                        return;
                    }
                    self.cursor += run;
                }
                b'\r' if self.bytes().get(self.cursor + 1) != Some(&b'\n') => {
                    let error_start = self.cursor;
                    self.cursor += 1;
                    valid = false;
                    self.problem(LexDiagnosticKind::InvalidLineBreak, error_start, self.cursor);
                }
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
            LexDiagnosticKind::UnterminatedString,
            start,
            self.source.len(),
        );
        self.push(SyntaxKind::MultiLineLiteralString, start, self.cursor);
        self.mark_last_error();
    }

    fn quote_run(&self, quote: u8) -> usize {
        let mut run = 0;
        while self.bytes().get(self.cursor + run) == Some(&quote) {
            run += 1;
        }
        run
    }

    fn scan_bare_atom(&mut self, start: usize) {
        self.cursor += 1;
        while self
            .bytes()
            .get(self.cursor)
            .is_some_and(|byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-'))
        {
            self.cursor += 1;
        }
        let kind = match &self.source[start..self.cursor] {
            "true" => SyntaxKind::True,
            "false" => SyntaxKind::False,
            "inf" => SyntaxKind::Inf,
            "nan" => SyntaxKind::Nan,
            _ => SyntaxKind::BareKey,
        };
        self.push(kind, start, self.cursor);
    }

    fn scan_digit_atom(&mut self, start: usize) {
        if self.is_time_shape(self.cursor) {
            self.scan_time(start);
            return;
        }
        if self.is_date_shape(self.cursor) {
            self.scan_date(start);
            return;
        }
        self.scan_number(start, false);
    }

    /// Classifies a sign-starting atom. `+` only ever begins a value, while
    /// `-` is also a bare-key character, so date-shaped and radix-prefixed
    /// runs after `-` stay bare keys (`-2024-01-01`, `-0x10`) instead of
    /// being misread as invalid numbers.
    fn scan_signed(&mut self, start: usize) {
        let sign = self.bytes()[start];
        let next = self.bytes().get(start + 1).copied();
        if next.is_some_and(|byte| byte.is_ascii_digit()) {
            let radix = self.bytes().get(start + 1) == Some(&b'0')
                && matches!(
                    self.bytes().get(start + 2),
                    Some(b'x') | Some(b'o') | Some(b'b')
                );
            if (sign == b'-' && radix) || (sign == b'-' && self.is_date_shape(start + 1)) {
                self.scan_bare_atom(start);
            } else {
                self.scan_number(start, true);
            }
            return;
        }
        if (sign == b'-' && self.is_signed_special(start))
            || (sign == b'+' && next.is_some_and(|byte| byte.is_ascii_alphabetic()))
        {
            self.scan_number(start, true);
            return;
        }
        self.scan_bare_atom(start);
    }

    fn is_signed_special(&self, start: usize) -> bool {
        let rest = &self.source[start..];
        (rest.starts_with("-inf") || rest.starts_with("-nan"))
            && !self
                .bytes()
                .get(start + 4)
                .is_some_and(|byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-'))
    }

    fn is_time_shape(&self, cursor: usize) -> bool {
        self.is_digit_run(cursor, 2)
            && self.bytes().get(cursor + 2) == Some(&b':')
            && self.is_digit_run(cursor + 3, 2)
    }

    fn is_date_shape(&self, cursor: usize) -> bool {
        self.is_digit_run(cursor, 4)
            && self.bytes().get(cursor + 4) == Some(&b'-')
            && self.is_digit_run(cursor + 5, 2)
            && self.bytes().get(cursor + 7) == Some(&b'-')
            && self.is_digit_run(cursor + 8, 2)
    }

    fn is_digit_run(&self, cursor: usize, count: usize) -> bool {
        (0..count).all(|offset| {
            self.bytes()
                .get(cursor + offset)
                .is_some_and(u8::is_ascii_digit)
        })
    }

    fn is_offset_shape(&self, cursor: usize) -> bool {
        self.is_digit_run(cursor + 1, 2)
            && self.bytes().get(cursor + 3) == Some(&b':')
            && self.is_digit_run(cursor + 4, 2)
    }

    fn scan_time(&mut self, start: usize) {
        self.scan_time_components();
        self.push(SyntaxKind::LocalTime, start, self.cursor);
    }

    fn scan_date(&mut self, start: usize) {
        self.validate_date_components(start);
        self.cursor = start + 10;
        let mut kind = SyntaxKind::LocalDate;
        if self
            .bytes()
            .get(self.cursor)
            .is_some_and(|byte| matches!(*byte, b'T' | b't' | b' '))
            && self.is_time_shape(self.cursor + 1)
        {
            self.cursor += 1;
            self.scan_time_components();
            kind = SyntaxKind::LocalDateTime;
            match self.bytes().get(self.cursor) {
                Some(b'Z') | Some(b'z') => {
                    self.cursor += 1;
                    kind = SyntaxKind::OffsetDateTime;
                }
                Some(b'+') | Some(b'-') if self.is_offset_shape(self.cursor) => {
                    self.scan_offset_components();
                    kind = SyntaxKind::OffsetDateTime;
                }
                _ => {}
            }
        }
        self.push(kind, start, self.cursor);
    }

        fn scan_time_components(&mut self) {
        let hour = self.component_span(2);
        let hour_value = self.component_value(hour);
        if hour_value > 23 {
            self.problem(
                LexDiagnosticKind::InvalidDateTime(DateTimeIssue::InvalidComponent),
                hour.0,
                hour.1,
            );
            self.mark_after_push = true;
        }
        self.cursor += 1;
        let minute = self.component_span(2);
        let minute_value = self.component_value(minute);
        if minute_value > 59 {
            self.problem(
                LexDiagnosticKind::InvalidDateTime(DateTimeIssue::InvalidComponent),
                minute.0,
                minute.1,
            );
            self.mark_after_push = true;
        }
        // The shape check only guarantees `HH:MM`; seconds are mandatory in
        // TOML, so stop before slicing digits that may not exist.
        let seconds = if self.bytes().get(self.cursor) == Some(&b':') {
            self.cursor += 1;
            self.is_digit_run(self.cursor, 2)
        } else {
            false
        };
        if seconds {
            let second = self.component_span(2);
            let second_value = self.component_value(second);
            if second_value > 60 {
                self.problem(
                    LexDiagnosticKind::InvalidDateTime(DateTimeIssue::InvalidComponent),
                    second.0,
                    second.1,
                );
                self.mark_after_push = true;
            }
            if self.bytes().get(self.cursor) == Some(&b'.')
                && self.is_digit_run(self.cursor + 1, 1)
            {
                self.cursor += 1;
                while self
                    .bytes()
                    .get(self.cursor)
                    .is_some_and(u8::is_ascii_digit)
                {
                    self.cursor += 1;
                }
            }
        } else {
            let anchor = self.cursor - 1;
            self.problem(
                LexDiagnosticKind::InvalidDateTime(DateTimeIssue::MissingComponent),
                anchor,
                self.cursor,
            );
            self.mark_after_push = true;
        }
    }

    fn scan_offset_components(&mut self) {
        self.cursor += 1;
        let hour = self.component_span(2);
        if self.component_value(hour) > 23 {
            self.problem(
                LexDiagnosticKind::InvalidDateTime(DateTimeIssue::InvalidComponent),
                hour.0,
                hour.1,
            );
            self.mark_after_push = true;
        }
        self.cursor += 1;
        let minute = self.component_span(2);
        if self.component_value(minute) > 59 {
            self.problem(
                LexDiagnosticKind::InvalidDateTime(DateTimeIssue::InvalidComponent),
                minute.0,
                minute.1,
            );
            self.mark_after_push = true;
        }
    }

    fn component_span(&mut self, count: usize) -> (usize, usize) {
        let span = (self.cursor, self.cursor + count);
        self.cursor += count;
        span
    }fn component_value(&self, span: (usize, usize)) -> u32 {
        self.source[span.0..span.1]
            .bytes()
            .fold(0_u32, |value, byte| value * 10 + u32::from(byte - b'0'))
    }

    fn validate_date_components(&mut self, start: usize) {
        let mut invalid = false;
        let year = (start, start + 4);
        let month = (start + 5, start + 7);
        let day = (start + 8, start + 10);
        let year_value = self.component_value(year);
        let month_value = self.component_value(month);
        let day_value = self.component_value(day);
        if year_value == 0 {
            self.problem(
                LexDiagnosticKind::InvalidDateTime(DateTimeIssue::InvalidComponent),
                year.0,
                year.1,
            );
            invalid = true;
        }
        if month_value == 0 || month_value > 12 {
            self.problem(
                LexDiagnosticKind::InvalidDateTime(DateTimeIssue::InvalidComponent),
                month.0,
                month.1,
            );
            invalid = true;
        }
        let max_day = if (1..=12).contains(&month_value) {
            days_in_month(year_value, month_value)
        } else {
            31
        };
        if day_value == 0 || day_value > max_day {
            self.problem(
                LexDiagnosticKind::InvalidDateTime(DateTimeIssue::InvalidComponent),
                day.0,
                day.1,
            );
            invalid = true;
        }
        if invalid {
            self.mark_after_push = true;
        }
    }

    fn scan_number(&mut self, start: usize, signed: bool) {
        self.cursor = start;
        if signed {
            self.cursor += 1;
        }
        if self
            .bytes()
            .get(self.cursor)
            .is_some_and(|byte| byte.is_ascii_alphabetic())
        {
            self.scan_signed_special(start);
            return;
        }
        if self.bytes().get(self.cursor) == Some(&b'0')
            && matches!(
                self.bytes().get(self.cursor + 1),
                Some(b'x') | Some(b'o') | Some(b'b')
            )
        {
            self.scan_radix_number(start);
            if signed {
                self.mark_last_error();
                self.problem(
                    LexDiagnosticKind::InvalidNumber(NumberIssue::SignedRadix),
                    start,
                    start + 1,
                );
            }
            return;
        }
        let integer_start = self.cursor;
        while self
            .bytes()
            .get(self.cursor)
            .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_')
        {
            self.cursor += 1;
        }
        if self.cursor == integer_start {
            let end = self.cursor.max(start + 1);
            self.push(SyntaxKind::Error, start, end);
            self.mark_last_error();
            self.problem(LexDiagnosticKind::InvalidNumber(NumberIssue::MissingDigits), start, end);
            return;
        }
        let mut float = false;
        if self.bytes().get(self.cursor) == Some(&b'.')
            && self
                .bytes()
                .get(self.cursor + 1)
                .is_some_and(u8::is_ascii_digit)
        {
            float = true;
            self.cursor += 1;
            while self
                .bytes()
                .get(self.cursor)
                .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_')
            {
                self.cursor += 1;
            }
        }
        if self
            .bytes()
            .get(self.cursor)
            .is_some_and(|byte| matches!(byte, b'e' | b'E'))
        {
            float = true;
            self.cursor += 1;
            if matches!(self.bytes().get(self.cursor), Some(b'+' | b'-')) {
                self.cursor += 1;
            }
            let digits_start = self.cursor;
            while self
                .bytes()
                .get(self.cursor)
                .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_')
            {
                self.cursor += 1;
            }
            if self.cursor == digits_start {
                let end = self.cursor.max(digits_start + 1).min(self.source.len());
                self.push(SyntaxKind::Float, start, end);
                self.mark_last_error();
                self.problem(
                    LexDiagnosticKind::InvalidNumber(NumberIssue::MissingExponentDigits),
                    start,
                    end,
                );
                return;
            }
        }
        let end = self.cursor;
        self.push(if float { SyntaxKind::Float } else { SyntaxKind::Integer }, start, end);
        if let Err((issue, relative_start, relative_end)) =
            validate_number(&self.source[start..end], signed)
        {
            self.mark_last_error();
            self.problem(
                LexDiagnosticKind::InvalidNumber(issue),
                start + relative_start,
                start + relative_end,
            );
        }
    }

    fn scan_radix_number(&mut self, start: usize) {
        self.cursor += 2;
        let digits_start = self.cursor;
        while self
            .bytes()
            .get(self.cursor)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            self.cursor += 1;
        }
        let end = self.cursor;
        let kind = match self.bytes()[digits_start - 1] {
            b'x' => SyntaxKind::HexInteger,
            b'o' => SyntaxKind::OctInteger,
            _ => SyntaxKind::BinInteger,
        };
        self.push(kind, start, end);
        let digits = &self.source[digits_start..end];
        if digits.is_empty() {
            self.mark_last_error();
            self.problem(
                LexDiagnosticKind::InvalidNumber(NumberIssue::MissingDigits),
                start,
                end,
            );
            return;
        }
        let radix = match kind {
            SyntaxKind::HexInteger => 16,
            SyntaxKind::OctInteger => 8,
            _ => 2,
        };
        if let Some(offset) = digits.bytes().position(|byte| {
            byte.is_ascii_alphanumeric() && digit_value(byte).is_none_or(|digit| digit >= radix)
        }) {
            self.mark_last_error();
            self.problem(
                LexDiagnosticKind::InvalidNumber(NumberIssue::InvalidRadixDigit),
                digits_start + offset,
                digits_start + offset + 1,
            );
            return;
        }
        if let Some(offset) = misplaced_underscore(digits, |byte| {
            digit_value(byte).is_some_and(|digit| digit < radix)
        }) {
            self.mark_last_error();
            self.problem(
                LexDiagnosticKind::InvalidNumber(NumberIssue::MisplacedUnderscore),
                digits_start + offset,
                digits_start + offset + 1,
            );
        }
    }

    fn scan_signed_special(&mut self, start: usize) {
        while self
            .bytes()
            .get(self.cursor)
            .is_some_and(|byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-'))
        {
            self.cursor += 1;
        }
        let special = matches!(
            &self.source[start..self.cursor],
            "+inf" | "+nan" | "-inf" | "-nan"
        );
        if special {
            let kind = if matches!(&self.source[start..self.cursor], "+inf" | "-inf") {
                SyntaxKind::Inf
            } else {
                SyntaxKind::Nan
            };
            self.push(kind, start, self.cursor);
        } else {
            self.push(SyntaxKind::Error, start, self.cursor);
            self.mark_last_error();
            self.problem(LexDiagnosticKind::UnexpectedCharacter, start + 1, self.cursor);
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
            flags: if std::mem::take(&mut self.mark_after_push) {
                TokenFlags::HAS_ERROR
            } else {
                TokenFlags::default()
            },
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

fn is_forbidden_control(byte: u8) -> bool {
    matches!(byte, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0x7F)
}

fn is_token_start(source: &str, cursor: usize) -> bool {
    matches!(
        source.as_bytes()[cursor],
        b' ' | b'\t'
            | b'\r'
            | b'\n'
            | b'#'
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'='
            | b','
            | b'.'
            | b'"'
            | b'\''
            | b'+'
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

fn digit_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn misplaced_underscore(digits: &str, is_digit: impl Fn(u8) -> bool) -> Option<usize> {
    let bytes = digits.as_bytes();
    bytes.iter().enumerate().find_map(|(index, &byte)| {
        if byte != b'_' {
            return None;
        }
        let previous_is_digit = index > 0 && is_digit(bytes[index - 1]);
        let next_is_digit = bytes.get(index + 1).is_some_and(|&byte| is_digit(byte));
        if previous_is_digit && next_is_digit {
            None
        } else {
            Some(index)
        }
    })
}

fn validate_number(text: &str, signed: bool) -> Result<(), (NumberIssue, usize, usize)> {
    let bytes = text.as_bytes();
    let mut cursor = if signed { 1 } else { 0 };
    let integer_start = cursor;
    while bytes.get(cursor).is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_') {
        cursor += 1;
    }
    let integer = &text[integer_start..cursor];
    if let Some(offset) = misplaced_underscore(integer, |byte| byte.is_ascii_digit()) {
        return Err((NumberIssue::MisplacedUnderscore, integer_start + offset, integer_start + offset + 1));
    }
    if integer.as_bytes().first() == Some(&b'0') && integer.len() > 1 {
        return Err((NumberIssue::LeadingZero, integer_start + 1, integer_start + 2));
    }
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let fraction_start = cursor;
        while bytes.get(cursor).is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_') {
            cursor += 1;
        }
        let fraction = &text[fraction_start..cursor];
        if let Some(offset) = misplaced_underscore(fraction, |byte| byte.is_ascii_digit()) {
            return Err((NumberIssue::MisplacedUnderscore, fraction_start + offset, fraction_start + offset + 1));
        }
    }
    if bytes.get(cursor).is_some_and(|byte| matches!(byte, b'e' | b'E')) {
        cursor += 1;
        if matches!(bytes.get(cursor), Some(b'+' | b'-')) {
            cursor += 1;
        }
        let exponent_start = cursor;
        while bytes.get(cursor).is_some_and(|byte| byte.is_ascii_digit() || *byte == b'_') {
            cursor += 1;
        }
        let exponent = &text[exponent_start..cursor];
        if let Some(offset) = misplaced_underscore(exponent, |byte| byte.is_ascii_digit()) {
            return Err((NumberIssue::MisplacedUnderscore, exponent_start + offset, exponent_start + offset + 1));
        }
    }
    Ok(())
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => 29,
        _ => 28,
    }
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
    fn emits_exact_toml_categories() {
        assert_eq!(
            kinds("[a.b] # t\nname = 'x'  # y\nflag = true\n"),
            vec![
                SyntaxKind::LeftBracket,
                SyntaxKind::BareKey,
                SyntaxKind::Dot,
                SyntaxKind::BareKey,
                SyntaxKind::RightBracket,
                SyntaxKind::Whitespace,
                SyntaxKind::LineComment,
                SyntaxKind::Newline,
                SyntaxKind::BareKey,
                SyntaxKind::Whitespace,
                SyntaxKind::Equals,
                SyntaxKind::Whitespace,
                SyntaxKind::LiteralString,
                SyntaxKind::Whitespace,
                SyntaxKind::LineComment,
                SyntaxKind::Newline,
                SyntaxKind::BareKey,
                SyntaxKind::Whitespace,
                SyntaxKind::Equals,
                SyntaxKind::Whitespace,
                SyntaxKind::True,
                SyntaxKind::Newline,
            ]
        );
        assert!(lex("[a.b] # t\nname = 'x'  # y\nflag = true\n")
            .diagnostics()
            .is_empty());
        assert_eq!(
            kinds("v = [1, {k = 2.5}]"),
            vec![
                SyntaxKind::BareKey,
                SyntaxKind::Whitespace,
                SyntaxKind::Equals,
                SyntaxKind::Whitespace,
                SyntaxKind::LeftBracket,
                SyntaxKind::Integer,
                SyntaxKind::Comma,
                SyntaxKind::Whitespace,
                SyntaxKind::LeftBrace,
                SyntaxKind::BareKey,
                SyntaxKind::Whitespace,
                SyntaxKind::Equals,
                SyntaxKind::Whitespace,
                SyntaxKind::Float,
                SyntaxKind::RightBrace,
                SyntaxKind::RightBracket,
            ]
        );
    }

    #[test]
    fn lexes_a_kitchen_sink_document_losslessly() {
        let source = concat!(
            "\u{feff}# score\r\n",
            "[player]\n",
            "name = 'Zoë 😀'\n",
            "id = 0xDE_ad_be\n",
            "hp = 1_000.000_1e-2\n",
            "born = 1997-09-21\n",
            "seen = 2024-02-29T07:32:00.999-08:00\n",
            "at = 07:32:00\n",
            "wins = true\n",
            "bio = \"\"\"\n",
            "line one\\\n",
            "  still one\n",
            "line two\"\"\"\n",
            "raw = '''\n",
            "no \\escapes\n",
            "'''\n",
            "[[player.scores]]\n",
            "value = 0b1_0\n",
            "[player.settings]\n",
            "deep = { x = inf, y = nan }\n",
        );
        let lexed = lex(source);
        assert!(lexed.diagnostics().is_empty(), "{:?}", lexed.diagnostics());
        assert_lossless(source, &lexed);
        let kinds: Vec<_> = lexed
            .significant_tokens()
            .map(|token| token.kind)
            .collect();
        for expected in [
            SyntaxKind::OffsetDateTime,
            SyntaxKind::LocalDate,
            SyntaxKind::LocalTime,
            SyntaxKind::MultiLineBasicString,
            SyntaxKind::MultiLineLiteralString,
            SyntaxKind::HexInteger,
            SyntaxKind::BinInteger,
            SyntaxKind::Float,
            SyntaxKind::Inf,
            SyntaxKind::Nan,
        ] {
            assert!(kinds.contains(&expected), "missing {expected:?}");
        }
    }

    #[test]
    fn tokens_losslessly_cover_unicode_and_malformed_input() {
        for source in [
            "",
            "Москва = 1",
            "[[a]]\nx = \"unterminated\ny = [",
            "a = 01\n\"b\" = @\n",
            "\u{feff}# hi\r\n2024-13-45 = 1",
            "a = '''\r\\'''",
            "x = -\ny = +0x1",
        ] {
            assert_lossless(source, &lex(source));
        }
    }

    #[test]
    fn accepts_exact_number_grammar() {
        for source in [
            "0", "-0", "42", "-12.5", "1e9", "1E-9", "0.0e+0", "1_000", "3.141_592",
        ] {
            assert!(lex(source).diagnostics().is_empty(), "{source}");
        }
        for (source, issue, span) in [
            ("01", NumberIssue::LeadingZero, Span::new(1, 2)),
            ("x = 01", NumberIssue::LeadingZero, Span::new(5, 6)),
            ("00.1", NumberIssue::LeadingZero, Span::new(1, 2)),
            ("1__2", NumberIssue::MisplacedUnderscore, Span::new(1, 2)),
            ("1_", NumberIssue::MisplacedUnderscore, Span::new(1, 2)),
            ("1e_2", NumberIssue::MisplacedUnderscore, Span::new(2, 3)),
            ("1e", NumberIssue::MissingExponentDigits, Span::new(0, 2)),
            ("1e+", NumberIssue::MissingExponentDigits, Span::new(0, 3)),
        ] {
            let lexed = lex(source);
            assert_eq!(lexed.diagnostics()[0].kind, LexDiagnosticKind::InvalidNumber(issue), "{source}");
            assert_eq!(lexed.diagnostics()[0].span, span, "{source}");
        }
        assert!(lex("1e").tokens()[0].has_error());
        assert_eq!(kinds("1e"), vec![SyntaxKind::Float]);
        assert_eq!(kinds("_1"), vec![SyntaxKind::BareKey]);
        assert!(lex("_1").diagnostics().is_empty());
        assert!(lex("0.1").diagnostics().is_empty());
        assert!(lex("-0.5").diagnostics().is_empty());
    }

    #[test]
    fn validates_radix_numbers() {
        assert!(lex("0xDEAD_beef").diagnostics().is_empty());
        assert!(lex("0o7_6").diagnostics().is_empty());
        assert!(lex("0b1_0").diagnostics().is_empty());
        for (source, issue, span) in [
            ("0x", NumberIssue::MissingDigits, Span::new(0, 2)),
            ("0xg", NumberIssue::InvalidRadixDigit, Span::new(2, 3)),
            ("0b12", NumberIssue::InvalidRadixDigit, Span::new(3, 4)),
            ("0o8", NumberIssue::InvalidRadixDigit, Span::new(2, 3)),
            ("0x1_", NumberIssue::MisplacedUnderscore, Span::new(3, 4)),
        ] {
            let lexed = lex(source);
            assert_eq!(
                lexed.diagnostics()[0].kind,
                LexDiagnosticKind::InvalidNumber(issue),
                "{source}"
            );
            assert_eq!(lexed.diagnostics()[0].span, span, "{source}");
        }
        let signed_radix = lex("+0x10");
        assert_eq!(kinds("+0x10"), vec![SyntaxKind::HexInteger]);
        assert_eq!(
            signed_radix.diagnostics()[0].kind,
            LexDiagnosticKind::InvalidNumber(NumberIssue::SignedRadix)
        );
        assert_eq!(signed_radix.diagnostics()[0].span, Span::new(0, 1));
        assert!(signed_radix.tokens()[0].has_error());
    }

    #[test]
    fn validates_string_escapes() {
        assert!(lex(r#""\b\t\n\f\r\"\\ \u00E9 \U0001F600""#)
            .diagnostics()
            .is_empty());
        assert_eq!(
            lex(r#""\q""#).diagnostics()[0].kind,
            LexDiagnosticKind::InvalidEscape
        );
        assert_eq!(
            lex(r#""\u12""#).diagnostics()[0].kind,
            LexDiagnosticKind::InvalidUnicodeEscape
        );
        assert_eq!(
            lex(r#""\uZZZZ""#).diagnostics()[0].kind,
            LexDiagnosticKind::InvalidUnicodeEscape
        );
        assert_eq!(lex(r#""\q""#).diagnostics()[0].span, Span::new(1, 3));
    }

    #[test]
    fn control_characters_are_rejected_but_tabs_pass() {
        let string = lex("\"a\u{1}b\"");
        assert_eq!(
            string.diagnostics()[0].kind,
            LexDiagnosticKind::UnescapedControlCharacter
        );
        assert_eq!(string.diagnostics()[0].span, Span::new(2, 3));
        assert!(string.tokens()[0].has_error());
        assert!(lex("'a\tb'").diagnostics().is_empty());
        let comment = lex("# \u{7}\n");
        assert_eq!(
            comment.diagnostics()[0].kind,
            LexDiagnosticKind::UnescapedControlCharacter
        );
        assert!(lex("# tab\tok\n").diagnostics().is_empty());
        let del = lex("'a\u{7f}b'");
        assert!(del.has_errors());
    }

    #[test]
    fn unterminated_strings_recover_at_the_newline() {
        let single = lex("\"abc\nd = 1");
        assert_eq!(single.diagnostics()[0].kind, LexDiagnosticKind::UnterminatedString);
        assert!(single.tokens().iter().any(|token| token.kind == SyntaxKind::BareKey));
        assert_lossless("\"abc\nd = 1", &single);
        let multi = lex("\"\"\"never ends");
        assert_eq!(multi.diagnostics()[0].kind, LexDiagnosticKind::UnterminatedString);
        assert_lossless("\"\"\"never ends", &multi);
    }

    #[test]
    fn validates_datetime_components() {
        for (source, span) in [
            ("2024-13-01", Span::new(5, 7)),
            ("2024-02-30", Span::new(8, 10)),
            ("2023-02-29", Span::new(8, 10)),
            ("0000-01-01", Span::new(0, 4)),
            ("12:60:00", Span::new(3, 5)),
            ("12:34:61", Span::new(6, 8)),
        ] {
            let lexed = lex(source);
            assert_eq!(
                lexed.diagnostics()[0].kind,
                LexDiagnosticKind::InvalidDateTime(DateTimeIssue::InvalidComponent),
                "{source}"
            );
            assert_eq!(lexed.diagnostics()[0].span, span, "{source}");
            assert!(lexed.tokens()[0].has_error(), "{source}");
        }
        assert!(lex("2024-02-29").diagnostics().is_empty());
        assert!(lex("12:34:60").diagnostics().is_empty(), "leap second");
        let short = lex("07:32");
        assert_eq!(
            short.diagnostics()[0].kind,
            LexDiagnosticKind::InvalidDateTime(DateTimeIssue::MissingComponent)
        );
        assert_eq!(kinds("07:32"), vec![SyntaxKind::LocalTime]);
        assert_lossless("07:32", &short);
    }

    #[test]
    fn classifies_all_four_datetime_shapes() {
        assert_eq!(kinds("2024-01-01"), vec![SyntaxKind::LocalDate]);
        assert_eq!(
            kinds("2024-01-01T07:32:00"),
            vec![SyntaxKind::LocalDateTime]
        );
        assert_eq!(
            kinds("2024-01-01t07:32:00z"),
            vec![SyntaxKind::OffsetDateTime]
        );
        assert_eq!(
            kinds("2024-01-01 07:32:00"),
            vec![SyntaxKind::LocalDateTime]
        );
        assert_eq!(
            kinds("2024-01-01T07:32:00+05:30"),
            vec![SyntaxKind::OffsetDateTime]
        );
        assert_eq!(kinds("07:32:00"), vec![SyntaxKind::LocalTime]);
        assert!(lex("2024-01-01t07:32:00z").diagnostics().is_empty());
        assert!(lex("2024-01-01T07:32:00.5-00:60").has_errors());
    }

    #[test]
    fn lone_carriage_returns_are_invalid_line_breaks() {
        let value = lex("a = 1\r2");
        assert_eq!(
            value.diagnostics()[0].kind,
            LexDiagnosticKind::InvalidLineBreak
        );
        assert_eq!(
            kinds("a = 1\r2"),
            vec![
                SyntaxKind::BareKey,
                SyntaxKind::Whitespace,
                SyntaxKind::Equals,
                SyntaxKind::Whitespace,
                SyntaxKind::Integer,
                SyntaxKind::Error,
                SyntaxKind::Integer,
            ]
        );
        let multi = lex("\"\"\"\r\"\"\"");
        assert_eq!(
            multi.diagnostics()[0].kind,
            LexDiagnosticKind::InvalidLineBreak
        );
        assert_eq!(multi.tokens()[0].kind, SyntaxKind::MultiLineBasicString);
        assert!(multi.tokens()[0].has_error());
        assert!(lex("a = 1\r\n").diagnostics().is_empty());
    }

    #[test]
    fn signs_dispatch_between_values_and_bare_keys() {
        assert_eq!(kinds("-1"), vec![SyntaxKind::Integer]);
        assert_eq!(kinds("-1.5"), vec![SyntaxKind::Float]);
        assert_eq!(kinds("-inf"), vec![SyntaxKind::Inf]);
        assert_eq!(kinds("+nan"), vec![SyntaxKind::Nan]);
        assert_eq!(kinds("+1"), vec![SyntaxKind::Integer]);
        assert_eq!(kinds("-"), vec![SyntaxKind::BareKey]);
        assert_eq!(kinds("-infinity"), vec![SyntaxKind::BareKey]);
        assert_eq!(kinds("-0x10"), vec![SyntaxKind::BareKey]);
        assert_eq!(kinds("-2024-01-01"), vec![SyntaxKind::BareKey]);
        assert!(lex("-0x10").diagnostics().is_empty());
        assert!(lex("-2024-01-01").diagnostics().is_empty());
        let plus_word = lex("+foo");
        assert_eq!(plus_word.tokens()[0].kind, SyntaxKind::Error);
        assert_eq!(
            plus_word.diagnostics()[0].kind,
            LexDiagnosticKind::UnexpectedCharacter
        );
        assert_eq!(plus_word.diagnostics()[0].span, Span::new(1, 4));
        for source in ["x = -1", "x = -inf", "x = +nan"] {
            assert!(lex(source).diagnostics().is_empty(), "{source}");
        }
    }

    #[test]
    fn dotted_key_numbers_split_on_dots_without_leading_digits() {
        assert_eq!(
            kinds("1.2"),
            vec![SyntaxKind::Float]
        );
        assert_eq!(
            kinds("1.foo"),
            vec![SyntaxKind::Integer, SyntaxKind::Dot, SyntaxKind::BareKey]
        );
        assert_eq!(
            kinds("1.2.3"),
            vec![SyntaxKind::Float, SyntaxKind::Dot, SyntaxKind::Integer]
        );
        assert!(lex("1.2.3").diagnostics().is_empty());
    }

    #[test]
    fn quote_runs_close_multi_line_strings() {
        assert_eq!(
            kinds("\"\"\"\"\"\""),
            vec![SyntaxKind::MultiLineBasicString]
        );
        assert!(lex("\"\"\"\"\"\"").diagnostics().is_empty());
        let seven = lex("\"\"\"\"\"\"\"");
        assert_eq!(seven.tokens()[0].kind, SyntaxKind::MultiLineBasicString);
        assert_eq!(seven.tokens()[0].text("\"\"\"\"\"\"\""), Some("\"\"\"\"\"\"\""));
        assert!(seven.diagnostics().is_empty());
        let four = lex("\"\"\"\"");
        assert_eq!(
            four.diagnostics()[0].kind,
            LexDiagnosticKind::UnterminatedString
        );
        assert_lossless("\"\"\"\"", &four);
        assert_eq!(
            kinds("'''''"),
            vec![SyntaxKind::MultiLineLiteralString]
        );
        assert!(lex("'''''").has_errors());
    }

    #[test]
    fn line_ending_backslashes_trim_following_whitespace() {
        let source = "x = \"\"\"a\\\n   b\"\"\"";
        let lexed = lex(source);
        assert!(lexed.diagnostics().is_empty(), "{:?}", lexed.diagnostics());
        assert_eq!(lexed.tokens()[0].kind, SyntaxKind::BareKey);
        assert_eq!(lexed.tokens()[2].kind, SyntaxKind::Equals);
        let string = lexed.tokens()[4];
        assert_eq!(string.kind, SyntaxKind::MultiLineBasicString);
        assert_eq!(string.text(source), Some("\"\"\"a\\\n   b\"\"\""));
        let crlf = lex("x = \"\"\"a\\\r\n   b\"\"\"");
        assert!(crlf.diagnostics().is_empty());
        let literal = lex("x = '''\n\\still raw\n'''");
        assert!(literal.diagnostics().is_empty());
    }

    #[test]
    fn resource_limits_keep_output_lossless_and_bounded() {
        let noisy = format!("\"{}\"", "\u{1}".repeat(100));
        let diagnostics = lex_with(&noisy, LexerOptions::new().max_diagnostics(3));
        assert_eq!(diagnostics.diagnostics().len(), 4);
        assert_eq!(
            diagnostics.diagnostics().last().unwrap().kind,
            LexDiagnosticKind::TooManyDiagnostics
        );
        assert!(diagnostics.is_truncated());

        let source = "a = [1, 2, 3, 4]";
        let limited = lex_with(source, LexerOptions::new().max_tokens(5));
        assert!(limited.is_truncated());
        assert!(limited.tokens().len() <= 6);
        assert_lossless(source, &limited);

        let big = "{\"x\":1}".repeat(100);
        let input_limited = lex_with(&big, LexerOptions::new().max_input_bytes(16));
        assert!(input_limited.is_truncated());
        assert_eq!(input_limited.tokens().len(), 1);
        assert_eq!(
            input_limited.diagnostics()[0].kind,
            LexDiagnosticKind::InputLimitExceeded
        );
    }

    #[test]
    fn bom_is_tolerated_once_and_newlines_are_significant() {
        let source = "\u{feff}a = 1\r\n";
        let lexed = lex(source);
        assert!(lexed.diagnostics().is_empty());
        assert_eq!(lexed.tokens()[0].kind, SyntaxKind::Bom);
        assert_eq!(lexed.tokens().last().unwrap().kind, SyntaxKind::Newline);
        let double = lex("\u{feff}\u{feff}");
        assert!(double.has_errors());
        assert_lossless("\u{feff}\u{feff}", &double);
    }

    #[test]
    fn booleans_and_special_floats_are_keywords() {
        assert_eq!(
            kinds("inf nan true false"),
            vec![
                SyntaxKind::Inf,
                SyntaxKind::Whitespace,
                SyntaxKind::Nan,
                SyntaxKind::Whitespace,
                SyntaxKind::True,
                SyntaxKind::Whitespace,
                SyntaxKind::False,
            ]
        );
        assert_eq!(
            kinds("infx"),
            vec![SyntaxKind::BareKey]
        );
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
}
