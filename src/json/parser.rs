//! Recovering recursive-descent parser for strict JSON and JSONC.

use std::{borrow::Cow, error::Error, fmt};

use crate::Span;

use super::{
    ast::{Array, Boolean, Member, Null, Number, Object, StringValue, Value},
    lexer::{LexDiagnosticKind, LexToken, Lexed, LexerOptions, SyntaxKind, lex_with},
};

/// Hard ceiling that keeps recursive parsing and AST destruction stack-safe.
pub const MAX_SUPPORTED_DEPTH: usize = 128;

/// Parser configuration with conservative resource limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParseOptions {
    lexer: LexerOptions,
    allow_trailing_commas: bool,
    max_depth: usize,
    max_diagnostics: usize,
}

impl ParseOptions {
    #[must_use]
    pub const fn strict() -> Self {
        Self {
            lexer: LexerOptions::strict().max_diagnostics(128),
            allow_trailing_commas: false,
            max_depth: 128,
            max_diagnostics: 128,
        }
    }

    #[must_use]
    pub const fn jsonc() -> Self {
        Self {
            lexer: LexerOptions::jsonc().max_diagnostics(128),
            allow_trailing_commas: true,
            max_depth: 128,
            max_diagnostics: 128,
        }
    }

    #[must_use]
    pub const fn allow_comments(mut self, yes: bool) -> Self {
        self.lexer = self.lexer.allow_comments(yes);
        self
    }

    #[must_use]
    pub const fn allow_bom(mut self, yes: bool) -> Self {
        self.lexer = self.lexer.allow_bom(yes);
        self
    }

    #[must_use]
    pub const fn allow_trailing_commas(mut self, yes: bool) -> Self {
        self.allow_trailing_commas = yes;
        self
    }

    /// Sets the maximum number of recursively nested arrays/objects.
    #[must_use]
    pub const fn max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = if max_depth > MAX_SUPPORTED_DEPTH {
            MAX_SUPPORTED_DEPTH
        } else {
            max_depth
        };
        self
    }

    /// Sets the number of detailed diagnostics retained before one truncation
    /// marker is emitted.
    #[must_use]
    pub const fn max_diagnostics(mut self, max_diagnostics: usize) -> Self {
        self.max_diagnostics = max_diagnostics;
        self.lexer = self.lexer.max_diagnostics(max_diagnostics);
        self
    }

    /// Bounds detailed lexical tokens. One final lossless error token may be
    /// appended to cover the unlexed suffix.
    #[must_use]
    pub const fn max_tokens(mut self, max_tokens: usize) -> Self {
        self.lexer = self.lexer.max_tokens(max_tokens);
        self
    }

    /// Bounds total UTF-8 input bytes before lexing or decoding begins.
    #[must_use]
    pub const fn max_input_bytes(mut self, max_input_bytes: usize) -> Self {
        self.lexer = self.lexer.max_input_bytes(max_input_bytes);
        self
    }

    #[must_use]
    pub const fn lexer_options(self) -> LexerOptions {
        self.lexer
    }

    #[must_use]
    pub const fn trailing_commas_allowed(self) -> bool {
        self.allow_trailing_commas
    }

    #[must_use]
    pub const fn depth_limit(self) -> usize {
        self.max_depth
    }

    #[must_use]
    pub const fn diagnostic_limit(self) -> usize {
        self.max_diagnostics
    }
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self::strict()
    }
}

/// A structural or decoded-string error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParseDiagnosticKind {
    Lexical(LexDiagnosticKind),
    ExpectedValue,
    ExpectedObjectKey,
    ExpectedColon,
    ExpectedCommaOrEnd,
    ExpectedArrayEnd,
    ExpectedObjectEnd,
    TrailingCommaNotAllowed,
    ExtraRootValue,
    UnexpectedClosingDelimiter,
    NestingLimitExceeded,
    InvalidUnicodeSurrogate,
    TooManyDiagnostics,
}

impl ParseDiagnosticKind {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Lexical(kind) => kind.code(),
            Self::ExpectedValue => "expected-value",
            Self::ExpectedObjectKey => "expected-object-key",
            Self::ExpectedColon => "expected-colon",
            Self::ExpectedCommaOrEnd => "expected-comma-or-end",
            Self::ExpectedArrayEnd => "expected-array-end",
            Self::ExpectedObjectEnd => "expected-object-end",
            Self::TrailingCommaNotAllowed => "trailing-comma-not-allowed",
            Self::ExtraRootValue => "extra-root-value",
            Self::UnexpectedClosingDelimiter => "unexpected-closing-delimiter",
            Self::NestingLimitExceeded => "nesting-limit-exceeded",
            Self::InvalidUnicodeSurrogate => "invalid-unicode-surrogate",
            Self::TooManyDiagnostics => "too-many-diagnostics",
        }
    }
}

impl fmt::Display for ParseDiagnosticKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lexical(kind) => kind.fmt(formatter),
            Self::ExpectedValue => formatter.write_str("expected a JSON value"),
            Self::ExpectedObjectKey => formatter.write_str("expected a quoted object key"),
            Self::ExpectedColon => formatter.write_str("expected `:` after the object key"),
            Self::ExpectedCommaOrEnd => {
                formatter.write_str("expected a comma or the end of the container")
            }
            Self::ExpectedArrayEnd => formatter.write_str("expected `]` to close the array"),
            Self::ExpectedObjectEnd => formatter.write_str("expected `}` to close the object"),
            Self::TrailingCommaNotAllowed => {
                formatter.write_str("trailing commas are not allowed in strict JSON")
            }
            Self::ExtraRootValue => formatter.write_str("expected exactly one root JSON value"),
            Self::UnexpectedClosingDelimiter => formatter.write_str("unexpected closing delimiter"),
            Self::NestingLimitExceeded => formatter.write_str("JSON nesting limit exceeded"),
            Self::InvalidUnicodeSurrogate => {
                formatter.write_str("invalid UTF-16 surrogate pair in JSON string")
            }
            Self::TooManyDiagnostics => formatter.write_str("additional diagnostics were omitted"),
        }
    }
}

/// A parser diagnostic with a UTF-8 byte span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ParseDiagnostic {
    pub kind: ParseDiagnosticKind,
    pub span: Span,
}

impl fmt::Display for ParseDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}..{}",
            self.kind, self.span.start, self.span.end
        )
    }
}

impl Error for ParseDiagnostic {}

/// Recovering parse output. It always retains the lossless token stream and
/// returns a root value whenever one could be constructed.
#[derive(Debug, Clone, PartialEq)]
pub struct Parse<'source> {
    lexed: Lexed<'source>,
    value: Option<Value<'source>>,
    diagnostics: Vec<ParseDiagnostic>,
    property_spans: Vec<Span>,
    invalid_value_spans: Vec<Span>,
}

impl<'source> Parse<'source> {
    #[must_use]
    pub fn source(&self) -> &'source str {
        self.lexed.source()
    }

    #[must_use]
    pub const fn value(&self) -> Option<&Value<'source>> {
        self.value.as_ref()
    }

    #[must_use]
    pub fn into_value(self) -> Option<Value<'source>> {
        self.value
    }

    #[must_use]
    pub const fn lexed(&self) -> &Lexed<'source> {
        &self.lexed
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[ParseDiagnostic] {
        &self.diagnostics
    }

    /// Spans of strings parsed in object-key position, including incomplete
    /// members whose value could not be recovered.
    #[must_use]
    pub fn property_spans(&self) -> &[Span] {
        &self.property_spans
    }

    /// Token spans whose decoded AST value is invalid even if diagnostics were
    /// truncated (currently malformed JSON strings/surrogate pairs).
    #[must_use]
    pub fn invalid_value_spans(&self) -> &[Span] {
        &self.invalid_value_spans
    }

    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty()
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.value.is_some() && self.diagnostics.is_empty()
    }
}

/// Parses strict JSON with bounded recovery.
#[must_use]
pub fn parse(source: &str) -> Parse<'_> {
    parse_with(source, ParseOptions::strict())
}

/// Parses JSON or JSONC with explicit options.
#[must_use]
pub fn parse_with(source: &str, options: ParseOptions) -> Parse<'_> {
    let lexed = lex_with(source, options.lexer);
    let mut parser = Parser {
        source,
        tokens: lexed.tokens(),
        cursor: 0,
        options,
        diagnostics: Vec::new(),
        diagnostics_truncated: false,
        truncation_offset: None,
        property_spans: Vec::new(),
        invalid_value_spans: Vec::new(),
    };
    for diagnostic in lexed.diagnostics() {
        if diagnostic.kind == LexDiagnosticKind::TooManyDiagnostics {
            parser.note_truncation(diagnostic.span.start);
            continue;
        }
        parser.problem(
            ParseDiagnosticKind::Lexical(diagnostic.kind),
            diagnostic.span,
        );
    }
    let value = parser.parse_document();
    parser.finalize_diagnostics();
    let diagnostics = parser.diagnostics;
    let property_spans = parser.property_spans;
    let invalid_value_spans = parser.invalid_value_spans;
    Parse {
        lexed,
        value,
        diagnostics,
        property_spans,
        invalid_value_spans,
    }
}

struct Parser<'source, 'tokens> {
    source: &'source str,
    tokens: &'tokens [LexToken],
    cursor: usize,
    options: ParseOptions,
    diagnostics: Vec<ParseDiagnostic>,
    diagnostics_truncated: bool,
    truncation_offset: Option<usize>,
    property_spans: Vec<Span>,
    invalid_value_spans: Vec<Span>,
}

impl<'source> Parser<'source, '_> {
    fn parse_document(&mut self) -> Option<Value<'source>> {
        let mut saw_token = false;
        let value = loop {
            let Some(token) = self.current() else {
                if !saw_token {
                    self.problem_at_cursor(ParseDiagnosticKind::ExpectedValue);
                }
                return None;
            };
            saw_token = true;
            let before = self.cursor;
            if let Some(value) = self.parse_value(0) {
                break Some(value);
            }
            if self.cursor == before {
                self.problem(ParseDiagnosticKind::UnexpectedClosingDelimiter, token.span);
                self.bump();
            }
        };

        while let Some(token) = self.current() {
            if matches!(
                token.kind,
                SyntaxKind::RightBrace | SyntaxKind::RightBracket
            ) {
                self.problem(ParseDiagnosticKind::UnexpectedClosingDelimiter, token.span);
                self.bump();
            } else {
                if token.kind.can_start_value() {
                    self.problem(ParseDiagnosticKind::ExtraRootValue, token.span);
                }
                let before = self.cursor;
                self.parse_value(0);
                if self.cursor == before {
                    self.bump();
                }
            }
        }
        value
    }

    fn parse_value(&mut self, depth: usize) -> Option<Value<'source>> {
        let Some(token) = self.current() else {
            self.problem_at_cursor(ParseDiagnosticKind::ExpectedValue);
            return None;
        };
        match token.kind {
            SyntaxKind::LeftBrace => {
                if depth >= self.options.max_depth {
                    self.problem(ParseDiagnosticKind::NestingLimitExceeded, token.span);
                    self.skip_container();
                    None
                } else {
                    Some(Value::Object(self.parse_object(depth + 1)))
                }
            }
            SyntaxKind::LeftBracket => {
                if depth >= self.options.max_depth {
                    self.problem(ParseDiagnosticKind::NestingLimitExceeded, token.span);
                    self.skip_container();
                    None
                } else {
                    Some(Value::Array(self.parse_array(depth + 1)))
                }
            }
            SyntaxKind::String => {
                self.bump();
                Some(Value::String(self.string_value(token)))
            }
            SyntaxKind::Number => {
                self.bump();
                Some(Value::Number(Number::new(
                    &self.source[token.span.range()],
                    token.span,
                    !token.has_error(),
                )))
            }
            SyntaxKind::True | SyntaxKind::False => {
                self.bump();
                Some(Value::Boolean(Boolean::new(
                    token.kind == SyntaxKind::True,
                    token.span,
                )))
            }
            SyntaxKind::Null => {
                self.bump();
                Some(Value::Null(Null::new(token.span)))
            }
            SyntaxKind::RightBrace | SyntaxKind::RightBracket => {
                self.problem(
                    ParseDiagnosticKind::ExpectedValue,
                    Span::new(token.span.start, token.span.start),
                );
                None
            }
            _ => {
                self.problem(ParseDiagnosticKind::ExpectedValue, token.span);
                self.bump();
                None
            }
        }
    }

    fn parse_array(&mut self, depth: usize) -> Array<'source> {
        let open = self.bump().expect("parse_array starts at `[` token");
        let mut elements = Vec::new();
        let mut end = open.span.end;
        let mut after_comma = false;

        loop {
            let Some(token) = self.current() else {
                self.problem_at_cursor(ParseDiagnosticKind::ExpectedArrayEnd);
                break;
            };
            if token.kind == SyntaxKind::RightBracket {
                let close = self.bump().expect("current token exists");
                if after_comma && !elements.is_empty() && !self.options.allow_trailing_commas {
                    self.problem(
                        ParseDiagnosticKind::TrailingCommaNotAllowed,
                        Span::new(end.saturating_sub(1), end),
                    );
                }
                end = close.span.end;
                break;
            }
            if token.kind == SyntaxKind::RightBrace {
                self.problem(ParseDiagnosticKind::ExpectedArrayEnd, token.span);
                end = self.bump().expect("current token exists").span.end;
                break;
            }
            if token.kind == SyntaxKind::Comma {
                self.problem(ParseDiagnosticKind::ExpectedValue, token.span);
                end = self.bump().expect("current token exists").span.end;
                after_comma = true;
                continue;
            }

            let before = self.cursor;
            if let Some(value) = self.parse_value(depth) {
                end = value.span().end;
                elements.push(value);
            }
            if self.cursor == before {
                // A mismatched closer belongs to the parent; do not swallow it.
                break;
            }
            after_comma = false;

            loop {
                let Some(next) = self.current() else {
                    self.problem_at_cursor(ParseDiagnosticKind::ExpectedArrayEnd);
                    return Array::new(elements, Span::new(open.span.start, end));
                };
                match next.kind {
                    SyntaxKind::Comma => {
                        end = self.bump().expect("current token exists").span.end;
                        after_comma = true;
                        break;
                    }
                    SyntaxKind::RightBracket => break,
                    SyntaxKind::RightBrace => {
                        self.problem(ParseDiagnosticKind::ExpectedArrayEnd, next.span);
                        end = self.bump().expect("current token exists").span.end;
                        return Array::new(elements, Span::new(open.span.start, end));
                    }
                    kind if kind.can_start_value() => {
                        self.problem(
                            ParseDiagnosticKind::ExpectedCommaOrEnd,
                            Span::new(next.span.start, next.span.start),
                        );
                        break;
                    }
                    _ => {
                        self.problem(ParseDiagnosticKind::ExpectedCommaOrEnd, next.span);
                        end = self.bump().expect("current token exists").span.end;
                    }
                }
            }
        }

        Array::new(elements, Span::new(open.span.start, end))
    }

    fn parse_object(&mut self, depth: usize) -> Object<'source> {
        let open = self.bump().expect("parse_object starts at `{` token");
        let mut members = Vec::new();
        let mut end = open.span.end;
        let mut after_comma = false;

        loop {
            let Some(token) = self.current() else {
                self.problem_at_cursor(ParseDiagnosticKind::ExpectedObjectEnd);
                break;
            };
            if token.kind == SyntaxKind::RightBrace {
                let close = self.bump().expect("current token exists");
                if after_comma && !members.is_empty() && !self.options.allow_trailing_commas {
                    self.problem(
                        ParseDiagnosticKind::TrailingCommaNotAllowed,
                        Span::new(end.saturating_sub(1), end),
                    );
                }
                end = close.span.end;
                break;
            }
            if token.kind == SyntaxKind::RightBracket {
                self.problem(ParseDiagnosticKind::ExpectedObjectEnd, token.span);
                end = self.bump().expect("current token exists").span.end;
                break;
            }
            if token.kind == SyntaxKind::Comma {
                self.problem(ParseDiagnosticKind::ExpectedObjectKey, token.span);
                end = self.bump().expect("current token exists").span.end;
                after_comma = true;
                continue;
            }
            if token.kind != SyntaxKind::String {
                self.problem(ParseDiagnosticKind::ExpectedObjectKey, token.span);
                self.recover_to_member();
                end = self.previous_end().max(end);
                after_comma = false;
                continue;
            }

            let key_token = self.bump().expect("current token exists");
            self.property_spans.push(key_token.span);
            let key = self.string_value(key_token);
            end = key_token.span.end;
            let has_colon = self.consume(SyntaxKind::Colon).is_some();
            if !has_colon {
                self.problem_at_cursor(ParseDiagnosticKind::ExpectedColon);
            } else {
                end = self.previous_end().max(end);
            }

            if !has_colon
                && self.current().is_some_and(|next| {
                    next.kind == SyntaxKind::String
                        && self.next_significant_kind() == Some(SyntaxKind::Colon)
                })
            {
                self.problem_at_cursor(ParseDiagnosticKind::ExpectedValue);
                continue;
            }

            // Treat a comma after `key:` as the member separator as well as a
            // missing-value anchor. Consuming it inside `parse_value` would
            // make the following valid member look like it lacked a comma.
            if self
                .current()
                .is_some_and(|next| next.kind == SyntaxKind::Comma)
            {
                let comma = self.bump().expect("current token exists");
                self.problem(ParseDiagnosticKind::ExpectedValue, comma.span);
                end = comma.span.end;
                after_comma = true;
                continue;
            }

            let before = self.cursor;
            if let Some(value) = self.parse_value(depth) {
                end = value.span().end;
                let member_span = Span::new(key.span().start, value.span().end);
                members.push(Member::new(key, value, member_span));
            }
            if self.cursor == before {
                // A closer can terminate the object after a missing value.
                if !matches!(
                    self.current().map(|current| current.kind),
                    Some(SyntaxKind::RightBrace | SyntaxKind::RightBracket)
                ) {
                    self.bump();
                }
            }
            after_comma = false;

            loop {
                let Some(next) = self.current() else {
                    self.problem_at_cursor(ParseDiagnosticKind::ExpectedObjectEnd);
                    return Object::new(members, Span::new(open.span.start, end));
                };
                match next.kind {
                    SyntaxKind::Comma => {
                        end = self.bump().expect("current token exists").span.end;
                        after_comma = true;
                        break;
                    }
                    SyntaxKind::RightBrace => break,
                    SyntaxKind::RightBracket => {
                        self.problem(ParseDiagnosticKind::ExpectedObjectEnd, next.span);
                        end = self.bump().expect("current token exists").span.end;
                        return Object::new(members, Span::new(open.span.start, end));
                    }
                    SyntaxKind::String => {
                        self.problem(
                            ParseDiagnosticKind::ExpectedCommaOrEnd,
                            Span::new(next.span.start, next.span.start),
                        );
                        break;
                    }
                    _ => {
                        self.problem(ParseDiagnosticKind::ExpectedCommaOrEnd, next.span);
                        end = self.bump().expect("current token exists").span.end;
                    }
                }
            }
        }

        Object::new(members, Span::new(open.span.start, end))
    }

    fn string_value(&mut self, token: LexToken) -> StringValue<'source> {
        let raw = &self.source[token.span.range()];
        let value = match decode_string(raw, token.span.start) {
            Ok(value) => Some(value),
            Err(DecodeError::Invalid) => {
                self.invalid_value_spans.push(token.span);
                None
            }
            Err(DecodeError::Surrogate(span)) => {
                self.invalid_value_spans.push(token.span);
                self.problem(ParseDiagnosticKind::InvalidUnicodeSurrogate, span);
                None
            }
        };
        StringValue::new(raw, value, token.span)
    }

    fn recover_to_member(&mut self) {
        let mut nesting = 0_usize;
        loop {
            let Some(token) = self.current() else {
                return;
            };
            match token.kind {
                SyntaxKind::Comma if nesting == 0 => {
                    self.bump();
                    return;
                }
                SyntaxKind::RightBrace | SyntaxKind::RightBracket if nesting == 0 => return,
                SyntaxKind::String
                    if nesting == 0 && self.next_significant_kind() == Some(SyntaxKind::Colon) =>
                {
                    return;
                }
                SyntaxKind::LeftBrace | SyntaxKind::LeftBracket => {
                    nesting += 1;
                    self.bump();
                }
                SyntaxKind::RightBrace | SyntaxKind::RightBracket => {
                    nesting = nesting.saturating_sub(1);
                    self.bump();
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn skip_container(&mut self) {
        let mut balance = 0_usize;
        loop {
            let Some(token) = self.current() else {
                return;
            };
            match token.kind {
                SyntaxKind::LeftBrace | SyntaxKind::LeftBracket => balance += 1,
                SyntaxKind::RightBrace | SyntaxKind::RightBracket => {
                    balance = balance.saturating_sub(1);
                }
                _ => {}
            }
            self.bump();
            if balance == 0 {
                return;
            }
        }
    }

    fn current(&mut self) -> Option<LexToken> {
        while self
            .tokens
            .get(self.cursor)
            .is_some_and(|token| token.kind.is_trivia())
        {
            self.cursor += 1;
        }
        self.tokens.get(self.cursor).copied()
    }

    fn bump(&mut self) -> Option<LexToken> {
        let token = self.current()?;
        self.cursor += 1;
        Some(token)
    }

    fn consume(&mut self, kind: SyntaxKind) -> Option<LexToken> {
        if self.current().is_some_and(|token| token.kind == kind) {
            self.bump()
        } else {
            None
        }
    }

    fn previous_end(&self) -> usize {
        self.tokens
            .get(self.cursor.saturating_sub(1))
            .map_or(0, |token| token.span.end)
    }

    fn next_significant_kind(&self) -> Option<SyntaxKind> {
        self.tokens[self.cursor.saturating_add(1)..]
            .iter()
            .find(|token| !token.kind.is_trivia())
            .map(|token| token.kind)
    }

    fn problem_at_cursor(&mut self, kind: ParseDiagnosticKind) {
        let offset = self
            .current()
            .map_or(self.source.len(), |token| token.span.start);
        self.problem(kind, Span::new(offset, offset));
    }

    fn problem(&mut self, kind: ParseDiagnosticKind, span: Span) {
        let collection_limit = self
            .options
            .max_diagnostics
            .saturating_mul(2)
            .saturating_add(2);
        if self.diagnostics.len() < collection_limit {
            self.diagnostics.push(ParseDiagnostic { kind, span });
        } else {
            self.note_truncation(span.start);
        }
    }

    fn note_truncation(&mut self, offset: usize) {
        self.diagnostics_truncated = true;
        self.truncation_offset = Some(
            self.truncation_offset
                .map_or(offset, |current| current.min(offset)),
        );
    }

    fn finalize_diagnostics(&mut self) {
        self.diagnostics.sort_by_key(|diagnostic| {
            (
                diagnostic.span.start,
                diagnostic_priority(diagnostic.kind),
                diagnostic.span.end,
                diagnostic.kind.code(),
            )
        });
        self.diagnostics
            .dedup_by(|right, left| right.kind == left.kind && right.span == left.span);
        let omitted_at = self
            .diagnostics
            .get(self.options.max_diagnostics)
            .map(|diagnostic| diagnostic.span.start);
        if self.diagnostics.len() > self.options.max_diagnostics {
            self.diagnostics.truncate(self.options.max_diagnostics);
            if let Some(offset) = omitted_at {
                self.note_truncation(offset);
            }
        }
        if self.diagnostics_truncated {
            let offset = match (omitted_at, self.truncation_offset) {
                (Some(left), Some(right)) => left.min(right),
                (Some(offset), None) | (None, Some(offset)) => offset,
                (None, None) => self.source.len(),
            };
            self.diagnostics.push(ParseDiagnostic {
                kind: ParseDiagnosticKind::TooManyDiagnostics,
                span: Span::new(offset, offset),
            });
        }
    }
}

const fn diagnostic_priority(kind: ParseDiagnosticKind) -> u8 {
    match kind {
        ParseDiagnosticKind::Lexical(_) | ParseDiagnosticKind::InvalidUnicodeSurrogate => 0,
        ParseDiagnosticKind::TooManyDiagnostics => 2,
        _ => 1,
    }
}

enum DecodeError {
    Invalid,
    Surrogate(Span),
}

fn decode_string<'source>(
    raw: &'source str,
    absolute_start: usize,
) -> Result<Cow<'source, str>, DecodeError> {
    if raw.len() < 2 || !raw.starts_with('"') || !raw.ends_with('"') {
        return Err(DecodeError::Invalid);
    }
    let inner = &raw[1..raw.len() - 1];
    if !inner.as_bytes().contains(&b'\\') {
        if inner.bytes().any(|byte| byte <= 0x1f) {
            return Err(DecodeError::Invalid);
        }
        return Ok(Cow::Borrowed(inner));
    }

    let bytes = inner.as_bytes();
    let mut output = String::with_capacity(inner.len());
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] != b'\\' {
            if bytes[cursor] <= 0x1f {
                return Err(DecodeError::Invalid);
            }
            let character = inner[cursor..].chars().next().ok_or(DecodeError::Invalid)?;
            output.push(character);
            cursor += character.len_utf8();
            continue;
        }

        let escape_start = cursor;
        cursor += 1;
        let Some(&escaped) = bytes.get(cursor) else {
            return Err(DecodeError::Invalid);
        };
        cursor += 1;
        match escaped {
            b'"' => output.push('"'),
            b'\\' => output.push('\\'),
            b'/' => output.push('/'),
            b'b' => output.push('\u{0008}'),
            b'f' => output.push('\u{000c}'),
            b'n' => output.push('\n'),
            b'r' => output.push('\r'),
            b't' => output.push('\t'),
            b'u' => {
                let Some(first) = read_hex_quad(bytes, cursor) else {
                    return Err(DecodeError::Invalid);
                };
                cursor += 4;
                let scalar = if (0xd800..=0xdbff).contains(&first) {
                    if bytes.get(cursor) != Some(&b'\\') || bytes.get(cursor + 1) != Some(&b'u') {
                        return Err(DecodeError::Surrogate(Span::new(
                            absolute_start + 1 + escape_start,
                            absolute_start + 1 + cursor,
                        )));
                    }
                    let Some(second) = read_hex_quad(bytes, cursor + 2) else {
                        return Err(DecodeError::Surrogate(Span::new(
                            absolute_start + 1 + escape_start,
                            absolute_start + 1 + (cursor + 2).min(bytes.len()),
                        )));
                    };
                    if !(0xdc00..=0xdfff).contains(&second) {
                        return Err(DecodeError::Surrogate(Span::new(
                            absolute_start + 1 + escape_start,
                            absolute_start + 1 + (cursor + 6).min(bytes.len()),
                        )));
                    }
                    cursor += 6;
                    0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
                } else if (0xdc00..=0xdfff).contains(&first) {
                    return Err(DecodeError::Surrogate(Span::new(
                        absolute_start + 1 + escape_start,
                        absolute_start + 1 + cursor,
                    )));
                } else {
                    u32::from(first)
                };
                output.push(char::from_u32(scalar).ok_or(DecodeError::Invalid)?);
            }
            _ => return Err(DecodeError::Invalid),
        }
    }
    Ok(Cow::Owned(output))
}

fn read_hex_quad(bytes: &[u8], start: usize) -> Option<u16> {
    let digits = bytes.get(start..start.checked_add(4)?)?;
    let mut value = 0_u16;
    for byte in digits {
        value = value.checked_mul(16)? + u16::from(hex_value(*byte)?);
    }
    Some(value)
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(source: &str) -> Vec<&'static str> {
        parse(source)
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.kind.code())
            .collect()
    }

    #[test]
    fn parses_nested_values_and_decodes_strings() {
        let parsed = parse(r#"{"city":"Москва","emoji":"\uD83D\uDE00","n":-1.2e3}"#);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        let object = parsed.value().and_then(Value::as_object).unwrap();
        assert_eq!(object.get("city").and_then(Value::as_str), Some("Москва"));
        assert_eq!(object.get("emoji").and_then(Value::as_str), Some("😀"));
        assert_eq!(
            object
                .get("n")
                .and_then(Value::as_number)
                .and_then(|number| number.as_f64().ok()),
            Some(-1200.0)
        );
    }

    #[test]
    fn validates_structure_that_is_lexically_clean() {
        for (source, expected) in [
            ("[1 2]", "expected-comma-or-end"),
            (r#"{"a" 1}"#, "expected-colon"),
            (r#"{"a":}"#, "expected-value"),
            ("true false", "extra-root-value"),
            ("[1", "expected-array-end"),
        ] {
            assert!(
                codes(source).contains(&expected),
                "{source}: {:?}",
                codes(source)
            );
        }
    }

    #[test]
    fn strict_and_jsonc_options_differ_explicitly() {
        let source = "{/*c*/\"x\":1,}";
        assert!(parse(source).has_errors());
        assert!(parse_with(source, ParseOptions::jsonc()).is_valid());
        let strict_comments = parse_with(
            source,
            ParseOptions::strict()
                .allow_comments(true)
                .allow_trailing_commas(false),
        );
        assert_eq!(
            strict_comments.diagnostics()[0].kind,
            ParseDiagnosticKind::TrailingCommaNotAllowed
        );
    }

    #[test]
    fn reports_lone_surrogates_but_accepts_pairs() {
        assert!(parse(r#""\uD83D\uDE00""#).is_valid());
        assert!(codes(r#""\uD83D""#).contains(&"invalid-unicode-surrogate"));
        assert!(codes(r#""\uDE00""#).contains(&"invalid-unicode-surrogate"));
    }

    #[test]
    fn depth_and_diagnostic_limits_are_bounded() {
        let deep = "[[[[[[0]]]]]]";
        let parsed = parse_with(deep, ParseOptions::strict().max_depth(3));
        assert!(codes(deep).is_empty());
        assert!(
            parsed
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.kind == ParseDiagnosticKind::NestingLimitExceeded)
        );

        let noisy = parse_with(
            "[,,:,,] trailing",
            ParseOptions::strict().max_diagnostics(2),
        );
        assert!(noisy.diagnostics().len() <= 3);
        assert_eq!(
            noisy.diagnostics().last().unwrap().kind,
            ParseDiagnosticKind::TooManyDiagnostics
        );
    }

    #[test]
    fn duplicate_keys_and_source_spans_are_preserved() {
        let source = r#"{"x":1,"x":2}"#;
        let parsed = parse(source);
        let object = parsed.value().and_then(Value::as_object).unwrap();
        assert_eq!(object.get_all("x").count(), 2);
        assert_eq!(object.span().slice(source), Some(source));
    }

    #[test]
    fn recovery_keeps_a_root_after_leading_garbage() {
        let parsed = parse("@ {\"a\":1}");
        assert!(parsed.value().and_then(Value::as_object).is_some());
        assert!(
            parsed
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.kind != ParseDiagnosticKind::ExtraRootValue)
        );
    }

    #[test]
    fn missing_member_value_does_not_lose_its_separator() {
        let parsed = parse(r#"{"a":,"b":2}"#);
        let object = parsed.value().and_then(Value::as_object).unwrap();
        assert!(object.get("b").is_some());
        assert!(
            !parsed
                .diagnostics()
                .iter()
                .any(|diagnostic| { diagnostic.kind == ParseDiagnosticKind::ExpectedCommaOrEnd })
        );
    }

    #[test]
    fn mismatched_root_closer_has_a_single_specific_diagnostic() {
        let parsed = parse("[}");
        assert!(
            parsed
                .diagnostics()
                .iter()
                .any(|diagnostic| { diagnostic.kind == ParseDiagnosticKind::ExpectedArrayEnd })
        );
        assert!(
            !parsed
                .diagnostics()
                .iter()
                .any(|diagnostic| { diagnostic.kind == ParseDiagnosticKind::ExtraRootValue })
        );
    }

    #[test]
    fn diagnostics_are_source_ordered_before_truncation() {
        let parsed = parse_with("[, @]", ParseOptions::strict().max_diagnostics(1));
        assert_eq!(parsed.diagnostics()[0].span.start, 1);
        assert_eq!(
            parsed.diagnostics().last().unwrap().kind,
            ParseDiagnosticKind::TooManyDiagnostics
        );
    }

    #[test]
    fn malformed_numbers_cannot_be_converted_as_valid_ast_values() {
        for source in ["01", "1.", "1.e2", "-"] {
            let parsed = parse(source);
            let number = parsed.value().and_then(Value::as_number).unwrap();
            assert!(!number.is_valid(), "{source}");
            assert_eq!(
                number.as_f64(),
                Err(crate::json::NumberError::InvalidJsonNumber),
                "{source}"
            );
        }
    }

    #[test]
    fn truncation_keeps_the_earliest_omitted_position_and_specific_error() {
        let parsed = parse_with("[01,02]", ParseOptions::strict().max_diagnostics(1));
        assert_eq!(parsed.diagnostics().last().unwrap().span, Span::new(5, 5));

        let parsed = parse_with("@", ParseOptions::strict().max_diagnostics(1));
        assert_eq!(
            parsed.diagnostics()[0].kind,
            ParseDiagnosticKind::Lexical(LexDiagnosticKind::UnexpectedCharacter)
        );
    }

    #[test]
    fn hard_depth_ceiling_is_enforced_at_its_boundary() {
        let source = format!(
            "{}0{}",
            "[".repeat(MAX_SUPPORTED_DEPTH + 1),
            "]".repeat(MAX_SUPPORTED_DEPTH + 1)
        );
        let parsed = parse_with(&source, ParseOptions::strict().max_depth(usize::MAX));
        assert_eq!(parsed.lexed().source(), source);
        assert!(
            parsed
                .diagnostics()
                .iter()
                .any(|diagnostic| { diagnostic.kind == ParseDiagnosticKind::NestingLimitExceeded })
        );
    }

    #[test]
    fn recovery_does_not_hoist_nested_invalid_keys() {
        let parsed = parse(r#"{"a":1,{"b":2},"tail":999}"#);
        let object = parsed.value().and_then(Value::as_object).unwrap();
        assert!(object.get("b").is_none());
        assert!(object.get("tail").is_some());
    }

    #[test]
    fn mismatched_child_closer_preserves_parent_siblings() {
        let parsed = parse(r#"{"a":[1,2},"tail":999}"#);
        let object = parsed.value().and_then(Value::as_object).unwrap();
        assert!(object.get("a").is_some());
        assert!(object.get("tail").is_some());
    }

    #[test]
    fn obvious_member_after_missing_colon_is_not_used_as_a_value() {
        let parsed = parse(r#"{"a" "b":2,"tail":999}"#);
        let object = parsed.value().and_then(Value::as_object).unwrap();
        assert!(object.get("a").is_none());
        assert!(object.get("b").is_some());
        assert!(object.get("tail").is_some());
    }

    #[test]
    fn non_values_after_root_do_not_get_extra_root_diagnostics() {
        let parsed = parse("1,2");
        assert_eq!(
            parsed
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.kind == ParseDiagnosticKind::ExtraRootValue)
                .count(),
            1
        );
    }
}
