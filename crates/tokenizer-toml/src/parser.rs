//! Recovering parser producing a merged TOML document tree.

use std::{borrow::Cow, error::Error, fmt};

use crate::Span;

use super::{
    ast::{
        Array, ArrayOfTables, Boolean, DateTime, DateTimeKind, Document, Entry, Float, Integer,
        Key, KeySegment, StringKind, StringValue, Table, TableOrigin, Value,
    },
    lexer::{LexDiagnosticKind, LexToken, Lexed, LexerOptions, SyntaxKind, lex_with},
};

/// Hard ceiling that keeps recursive parsing stack-safe.
pub const MAX_SUPPORTED_DEPTH: usize = 128;

/// Parser configuration with conservative resource limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParseOptions {
    lexer: LexerOptions,
    max_depth: usize,
    max_diagnostics: usize,
}

impl ParseOptions {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            lexer: LexerOptions::new(),
            max_depth: MAX_SUPPORTED_DEPTH,
            max_diagnostics: 128,
        }
    }

    /// Sets the maximum number of recursively nested arrays and inline tables.
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

    /// Bounds total UTF-8 input bytes before lexing begins.
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
        Self::new()
    }
}

/// A structural or decoded-string error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParseDiagnosticKind {
    Lexical(LexDiagnosticKind),
    ExpectedKey,
    InvalidKey,
    ExpectedEquals,
    ExpectedValue,
    ExpectedHeaderEnd,
    ExpectedArrayEnd,
    ExpectedInlineTableEnd,
    ExpectedCommaOrEnd,
    ExpectedLineEnd,
    DuplicateKey,
    CannotExtendValue,
    CannotExtendInlineTable,
    InvalidUnicodeScalar,
    NestingLimitExceeded,
    TooManyDiagnostics,
}

impl ParseDiagnosticKind {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Lexical(kind) => kind.code(),
            Self::ExpectedKey => "expected-key",
            Self::InvalidKey => "invalid-key",
            Self::ExpectedEquals => "expected-equals",
            Self::ExpectedValue => "expected-value",
            Self::ExpectedHeaderEnd => "expected-header-end",
            Self::ExpectedArrayEnd => "expected-array-end",
            Self::ExpectedInlineTableEnd => "expected-inline-table-end",
            Self::ExpectedCommaOrEnd => "expected-comma-or-end",
            Self::ExpectedLineEnd => "expected-line-end",
            Self::DuplicateKey => "duplicate-key",
            Self::CannotExtendValue => "cannot-extend-value",
            Self::CannotExtendInlineTable => "cannot-extend-inline-table",
            Self::InvalidUnicodeScalar => "invalid-unicode-scalar",
            Self::NestingLimitExceeded => "nesting-limit-exceeded",
            Self::TooManyDiagnostics => "too-many-diagnostics",
        }
    }
}

impl fmt::Display for ParseDiagnosticKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lexical(kind) => kind.fmt(formatter),
            Self::ExpectedKey => formatter.write_str("expected a bare or quoted key"),
            Self::InvalidKey => formatter.write_str("this token is not a valid TOML key"),
            Self::ExpectedEquals => formatter.write_str("expected `=` after the key"),
            Self::ExpectedValue => formatter.write_str("expected a TOML value"),
            Self::ExpectedHeaderEnd => formatter.write_str("expected `]` to close the header"),
            Self::ExpectedArrayEnd => formatter.write_str("expected `]` to close the array"),
            Self::ExpectedInlineTableEnd => {
                formatter.write_str("expected `}` to close the inline table")
            }
            Self::ExpectedCommaOrEnd => {
                formatter.write_str("expected a comma or the end of the container")
            }
            Self::ExpectedLineEnd => {
                formatter.write_str("expected a line break after the expression")
            }
            Self::DuplicateKey => formatter.write_str("this key is already defined"),
            Self::CannotExtendValue => formatter.write_str("a value cannot be extended by a key"),
            Self::CannotExtendInlineTable => {
                formatter.write_str("an inline table cannot be extended by additional keys")
            }
            Self::InvalidUnicodeScalar => {
                formatter.write_str("invalid Unicode scalar value in TOML string")
            }
            Self::NestingLimitExceeded => formatter.write_str("TOML nesting limit exceeded"),
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

/// Recovering parse output. It always retains the lossless token stream and a
/// merged document holding every expression that could be recovered.
#[derive(Debug, Clone, PartialEq)]
pub struct Parse<'source> {
    lexed: Lexed<'source>,
    document: Document<'source>,
    diagnostics: Vec<ParseDiagnostic>,
}

impl<'source> Parse<'source> {
    #[must_use]
    pub fn source(&self) -> &'source str {
        self.lexed.source()
    }

    #[must_use]
    pub const fn document(&self) -> &Document<'source> {
        &self.document
    }

    #[must_use]
    pub fn into_document(self) -> Document<'source> {
        self.document
    }

    #[must_use]
    pub const fn lexed(&self) -> &Lexed<'source> {
        &self.lexed
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[ParseDiagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty()
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

/// Parses TOML with bounded recovery.
#[must_use]
pub fn parse(source: &str) -> Parse<'_> {
    parse_with(source, ParseOptions::new())
}

/// Parses TOML with explicit options.
#[must_use]
pub fn parse_with(source: &str, options: ParseOptions) -> Parse<'_> {
    let lexed = lex_with(source, options.lexer);
    parse_lexed(source, lexed, options)
}

fn parse_lexed<'source>(
    source: &'source str,
    lexed: Lexed<'source>,
    options: ParseOptions,
) -> Parse<'source> {
    let mut root = TableSlot::root();
    let mut parser = Parser {
        source,
        tokens: lexed.tokens(),
        cursor: 0,
        last_end: 0,
        options,
        current_path: Vec::new(),
        diagnostics: Vec::new(),
        diagnostics_truncated: false,
        truncation_offset: None,
        accepted_key_spans: Vec::new(),
    };
    parser.parse_document(&mut root);
    for diagnostic in lexed.diagnostics() {
        if diagnostic.kind == LexDiagnosticKind::TooManyDiagnostics {
            parser.note_truncation(diagnostic.span.start);
            continue;
        }
        let accepted = matches!(
            diagnostic.kind,
            LexDiagnosticKind::InputLimitExceeded | LexDiagnosticKind::TokenLimitExceeded
        ) || parser
            .accepted_key_spans
            .iter()
            .any(|span| span.start <= diagnostic.span.start && diagnostic.span.end <= span.end);
        if !accepted {
            parser.problem(
                ParseDiagnosticKind::Lexical(diagnostic.kind),
                diagnostic.span,
            );
        }
    }
    parser.finalize_diagnostics();
    let diagnostics = parser.diagnostics;
    let document = Document::new(convert_root(root, source.len()), Span::new(0, source.len()));
    Parse {
        lexed,
        document,
        diagnostics,
    }
}

/// Where an expression's keys are resolved from: headers are absolute, dotted
/// keys extend the current table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavMode {
    Header,
    Dotted,
}

#[derive(Debug, Clone, Copy)]
enum StepAction {
    Descend(usize),
    Create(Origin),
    Reject(ParseDiagnosticKind),
}

#[derive(Debug, Clone, Copy)]
enum HeaderAction {
    Upgrade(usize),
    Append(usize),
    CreateTable,
    CreateArray,
    Reject(ParseDiagnosticKind),
}

#[derive(Debug, Clone, Copy)]
enum Origin {
    Root,
    HeaderExplicit,
    HeaderImplicit,
    Dotted,
    Inline,
}

/// Intermediate table representation before conversion into the borrowed AST.
struct TableSlot<'source> {
    origin: Origin,
    header: Option<Span>,
    key_span: Span,
    entries: Vec<DefEntry<'source>>,
}

impl TableSlot<'_> {
    fn root() -> Self {
        Self {
            origin: Origin::Root,
            header: None,
            key_span: Span::new(0, 0),
            entries: Vec::new(),
        }
    }
}

struct DefEntry<'source> {
    /// The key segments as written for this expression, relative to the table
    /// the expression was resolved against.
    path: Vec<KeySegment<'source>>,
    name: String,
    key_span: Span,
    node: DefNode<'source>,
}

enum DefNode<'source> {
    Table(TableSlot<'source>),
    ArrayOfTables {
        elements: Vec<TableSlot<'source>>,
        header: Span,
        key_span: Span,
    },
    Leaf(Value<'source>),
}

fn slot_at<'source, 'slot>(
    root: &'slot TableSlot<'source>,
    path: &[usize],
) -> &'slot TableSlot<'source> {
    let mut slot = root;
    for &index in path {
        slot = match &slot.entries[index].node {
            DefNode::Table(table) => table,
            // Dotted keys after `[[a]]` always target the newest element.
            DefNode::ArrayOfTables { elements, .. } => {
                elements.last().expect("array of tables is never empty")
            }
            DefNode::Leaf(_) => unreachable!("paths only address tables"),
        };
    }
    slot
}

fn slot_at_mut<'source, 'slot>(
    root: &'slot mut TableSlot<'source>,
    path: &[usize],
) -> &'slot mut TableSlot<'source> {
    let mut slot = root;
    for &index in path {
        slot = match &mut slot.entries[index].node {
            DefNode::Table(table) => table,
            DefNode::ArrayOfTables { elements, .. } => {
                elements.last_mut().expect("array of tables is never empty")
            }
            DefNode::Leaf(_) => unreachable!("paths only address tables"),
        };
    }
    slot
}

fn segment_name(segment: &KeySegment<'_>) -> String {
    segment
        .decoded()
        .map_or_else(|| segment.raw().to_owned(), str::to_string)
}

fn is_bare_key_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn strip_first_newline(inner: &str) -> &str {
    if let Some(rest) = inner.strip_prefix("\r\n") {
        rest
    } else if let Some(rest) = inner.strip_prefix('\n') {
        rest
    } else {
        inner
    }
}

enum DecodeError {
    Invalid,
    NotScalar(Span),
}

fn decode_basic<'source>(
    raw: &'source str,
    absolute_start: usize,
) -> Result<Cow<'source, str>, DecodeError> {
    let Some(inner) = raw.get(1..raw.len().saturating_sub(1)) else {
        return Err(DecodeError::Invalid);
    };
    if !inner.as_bytes().contains(&b'\\') {
        return Ok(Cow::Borrowed(inner));
    }
    let mut output = String::new();
    push_basic_body(inner, absolute_start + 1, false, &mut output)?;
    Ok(Cow::Owned(output))
}

fn decode_multi_line_basic<'source>(
    raw: &'source str,
    absolute_start: usize,
) -> Result<Cow<'source, str>, DecodeError> {
    let Some(inner) = raw.get(3..raw.len().saturating_sub(3)) else {
        return Err(DecodeError::Invalid);
    };
    let stripped = strip_first_newline(inner);
    if !stripped.as_bytes().contains(&b'\\') {
        return Ok(Cow::Borrowed(stripped));
    }
    let mut output = String::new();
    push_basic_body(
        stripped,
        absolute_start + 3 + (inner.len() - stripped.len()),
        true,
        &mut output,
    )?;
    Ok(Cow::Owned(output))
}

fn push_basic_body(
    inner: &str,
    absolute_start: usize,
    continuation: bool,
    output: &mut String,
) -> Result<(), DecodeError> {
    let bytes = inner.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] != b'\\' {
            let start = cursor;
            cursor += 1;
            while cursor < bytes.len() && bytes[cursor] != b'\\' {
                cursor += 1;
            }
            output.push_str(&inner[start..cursor]);
            continue;
        }
        let escape_start = cursor;
        cursor += 1;
        if continuation {
            let mut after = cursor;
            while after < bytes.len() && matches!(bytes[after], b' ' | b'\t') {
                after += 1;
            }
            let line_feed = bytes.get(after) == Some(&b'\n');
            let carriage_feed =
                bytes.get(after) == Some(&b'\r') && bytes.get(after + 1) == Some(&b'\n');
            if line_feed || carriage_feed {
                cursor = after;
                if bytes[cursor] == b'\r' {
                    cursor += 1;
                }
                cursor += 1;
                while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t' | b'\r' | b'\n')
                {
                    cursor += 1;
                }
                continue;
            }
        }
        let Some(&escaped) = bytes.get(cursor) else {
            return Err(DecodeError::Invalid);
        };
        cursor += 1;
        match escaped {
            b'"' => output.push('"'),
            b'\\' => output.push('\\'),
            b'b' => output.push('\u{8}'),
            b'f' => output.push('\u{c}'),
            b'n' => output.push('\n'),
            b'r' => output.push('\r'),
            b't' => output.push('\t'),
            b'u' | b'U' => {
                let width = if escaped == b'u' { 4 } else { 8 };
                if cursor + width > bytes.len()
                    || !bytes[cursor..cursor + width]
                        .iter()
                        .all(u8::is_ascii_hexdigit)
                {
                    return Err(DecodeError::Invalid);
                }
                let mut value = 0_u32;
                for digit in &bytes[cursor..cursor + width] {
                    let digit = match digit {
                        b'0'..=b'9' => u32::from(digit - b'0'),
                        b'a'..=b'f' => u32::from(digit - b'a' + 10),
                        _ => u32::from(digit - b'A' + 10),
                    };
                    value = value * 16 + digit;
                }
                cursor += width;
                let Some(scalar) = char::from_u32(value) else {
                    return Err(DecodeError::NotScalar(Span::new(
                        absolute_start + escape_start,
                        absolute_start + cursor,
                    )));
                };
                output.push(scalar);
            }
            _ => return Err(DecodeError::Invalid),
        }
    }
    Ok(())
}

fn convert_root(slot: TableSlot<'_>, source_len: usize) -> Table<'_> {
    convert_slot_with(slot, Span::new(0, source_len))
}

fn convert_slot_with(slot: TableSlot<'_>, seed: Span) -> Table<'_> {
    let mut span = seed;
    let mut entries = Vec::with_capacity(slot.entries.len());
    for entry in slot.entries {
        let (value, value_end) = convert_node(entry.node);
        let start = entry
            .path
            .first()
            .map_or(entry.key_span.start, |segment| segment.span().start);
        let entry_span = Span::new(start, value_end.max(start));
        span = span.cover(entry_span);
        entries.push(Entry::new(build_key(entry.path), value, entry_span));
    }
    if let Some(header) = slot.header {
        span = span.cover(header);
    }
    let origin = match slot.origin {
        Origin::Root => TableOrigin::Root,
        Origin::HeaderExplicit | Origin::HeaderImplicit => TableOrigin::Header,
        Origin::Dotted => TableOrigin::Dotted,
        Origin::Inline => TableOrigin::Inline,
    };
    Table::new(entries, origin, slot.header, span)
}

fn convert_node(node: DefNode<'_>) -> (Value<'_>, usize) {
    match node {
        DefNode::Leaf(value) => {
            let end = value.span().end;
            (value, end)
        }
        DefNode::Table(table) => {
            let seed = table.header.unwrap_or(table.key_span);
            let table = convert_slot_with(table, seed);
            let end = table.span().end;
            (Value::Table(table), end)
        }
        DefNode::ArrayOfTables {
            elements,
            header,
            key_span,
        } => {
            let mut span = key_span.cover(header);
            let mut converted = Vec::with_capacity(elements.len());
            for element in elements {
                let seed = element.header.unwrap_or(element.key_span);
                let table = convert_slot_with(element, seed);
                span = span.cover(table.span());
                converted.push(table);
            }
            (
                Value::ArrayOfTables(ArrayOfTables::new(converted, span)),
                span.end,
            )
        }
    }
}

fn build_key(segments: Vec<KeySegment<'_>>) -> Key<'_> {
    let span = match (segments.first(), segments.last()) {
        (Some(first), Some(last)) => first.span().cover(last.span()),
        _ => Span::new(0, 0),
    };
    Key::new(segments, span)
}

struct Parser<'source, 'tokens> {
    source: &'source str,
    tokens: &'tokens [LexToken],
    cursor: usize,
    last_end: usize,
    options: ParseOptions,
    current_path: Vec<usize>,
    diagnostics: Vec<ParseDiagnostic>,
    diagnostics_truncated: bool,
    truncation_offset: Option<usize>,
    accepted_key_spans: Vec<Span>,
}

impl<'source> Parser<'source, '_> {
    fn parse_document(&mut self, root: &mut TableSlot<'source>) {
        loop {
            let Some(token) = self.current() else {
                return;
            };
            match token.kind {
                SyntaxKind::Newline => {
                    self.bump();
                }
                SyntaxKind::LeftBracket => {
                    self.parse_header(root);
                }
                kind if kind.can_start_key() => {
                    self.parse_key_value(root);
                }
                _ => {
                    self.problem(ParseDiagnosticKind::ExpectedKey, token.span);
                    self.bump();
                    self.recover_to_line_end();
                }
            }
        }
    }

    fn parse_header(&mut self, root: &mut TableSlot<'source>) {
        let open = self.bump().expect("parse_header starts at `[` token");
        let is_array = self.consume(SyntaxKind::LeftBracket);
        let Some(segments) = self.parse_key() else {
            self.recover_to_line_end();
            return;
        };
        if !self.consume(SyntaxKind::RightBracket) {
            self.problem_at_cursor(ParseDiagnosticKind::ExpectedHeaderEnd);
            self.recover_to_line_end();
            return;
        }
        if is_array && !self.consume(SyntaxKind::RightBracket) {
            self.problem_at_cursor(ParseDiagnosticKind::ExpectedHeaderEnd);
            self.recover_to_line_end();
            return;
        }
        let header_span = Span::new(open.span.start, self.last_end.max(open.span.start + 1));
        let last = segments
            .last()
            .expect("parse_key returns at least one segment");
        let key_span = last.span();
        let name = segment_name(last);
        let Some(nav_path) = self.navigate_intermediates(root, &segments, NavMode::Header) else {
            self.expect_line_end();
            return;
        };
        let action = {
            let slot = slot_at(root, &nav_path);
            match slot.entries.iter().position(|entry| entry.name == name) {
                Some(index) => match &slot.entries[index].node {
                    DefNode::Table(child) => match child.origin {
                        Origin::HeaderImplicit if !is_array => HeaderAction::Upgrade(index),
                        Origin::Inline => {
                            HeaderAction::Reject(ParseDiagnosticKind::CannotExtendInlineTable)
                        }
                        _ => HeaderAction::Reject(ParseDiagnosticKind::DuplicateKey),
                    },
                    DefNode::Leaf(_) => {
                        HeaderAction::Reject(ParseDiagnosticKind::CannotExtendValue)
                    }
                    DefNode::ArrayOfTables { .. } if is_array => HeaderAction::Append(index),
                    DefNode::ArrayOfTables { .. } => {
                        HeaderAction::Reject(ParseDiagnosticKind::DuplicateKey)
                    }
                },
                None if is_array => HeaderAction::CreateArray,
                None => HeaderAction::CreateTable,
            }
        };
        match action {
            HeaderAction::Upgrade(index) => {
                let slot = slot_at_mut(root, &nav_path);
                if let DefNode::Table(child) = &mut slot.entries[index].node {
                    child.origin = Origin::HeaderExplicit;
                    child.header = Some(header_span);
                }
                self.current_path = nav_path;
                self.current_path.push(index);
            }
            HeaderAction::Append(index) => {
                let slot = slot_at_mut(root, &nav_path);
                if let DefNode::ArrayOfTables { elements, .. } = &mut slot.entries[index].node {
                    elements.push(TableSlot {
                        origin: Origin::HeaderExplicit,
                        header: Some(header_span),
                        key_span: header_span,
                        entries: Vec::new(),
                    });
                }
                self.current_path = nav_path;
                self.current_path.push(index);
            }
            HeaderAction::CreateTable | HeaderAction::CreateArray => {
                let node = if matches!(action, HeaderAction::CreateArray) {
                    DefNode::ArrayOfTables {
                        elements: vec![TableSlot {
                            origin: Origin::HeaderExplicit,
                            header: Some(header_span),
                            key_span: header_span,
                            entries: Vec::new(),
                        }],
                        header: header_span,
                        key_span: header_span,
                    }
                } else {
                    DefNode::Table(TableSlot {
                        origin: Origin::HeaderExplicit,
                        header: Some(header_span),
                        key_span: header_span,
                        entries: Vec::new(),
                    })
                };
                let slot = slot_at_mut(root, &nav_path);
                slot.entries.push(DefEntry {
                    path: segments,
                    name,
                    key_span,
                    node,
                });
                self.current_path = nav_path;
                self.current_path.push(slot.entries.len() - 1);
            }
            HeaderAction::Reject(kind) => {
                self.problem(kind, header_span);
            }
        }
        self.expect_line_end();
    }

    fn parse_key_value(&mut self, root: &mut TableSlot<'source>) {
        let Some(segments) = self.parse_key() else {
            self.recover_to_line_end();
            return;
        };
        if !self.consume(SyntaxKind::Equals) {
            self.problem_at_cursor(ParseDiagnosticKind::ExpectedEquals);
            self.recover_to_line_end();
            return;
        }
        let Some(value) = self.parse_value(0) else {
            self.recover_to_line_end();
            return;
        };
        let base_path = self.current_path.clone();
        let last = segments
            .last()
            .expect("parse_key returns at least one segment");
        let key_span = last.span();
        let name = segment_name(last);
        let nav_path = {
            let base = slot_at_mut(root, &base_path);
            self.navigate_intermediates(base, &segments, NavMode::Dotted)
        };
        if let Some(nav_path) = nav_path {
            let duplicate = {
                let base = slot_at(root, &base_path);
                slot_at(base, &nav_path)
                    .entries
                    .iter()
                    .any(|entry| entry.name == name)
            };
            if duplicate {
                self.problem(ParseDiagnosticKind::DuplicateKey, key_span);
            } else {
                let base = slot_at_mut(root, &base_path);
                let target = slot_at_mut(base, &nav_path);
                target.entries.push(DefEntry {
                    path: segments,
                    name,
                    key_span,
                    node: DefNode::Leaf(value),
                });
            }
        }
        self.expect_line_end();
    }

    fn navigate_intermediates(
        &mut self,
        base: &mut TableSlot<'source>,
        segments: &[KeySegment<'source>],
        mode: NavMode,
    ) -> Option<Vec<usize>> {
        let mut path: Vec<usize> = Vec::new();
        for depth in 0..segments.len().saturating_sub(1) {
            let name = segment_name(&segments[depth]);
            let action = {
                let slot = slot_at(base, &path);
                match slot.entries.iter().position(|entry| entry.name == name) {
                    Some(index) => match &slot.entries[index].node {
                        DefNode::Table(child) => match child.origin {
                            Origin::Inline => {
                                StepAction::Reject(ParseDiagnosticKind::CannotExtendInlineTable)
                            }
                            _ => StepAction::Descend(index),
                        },
                        DefNode::ArrayOfTables { .. } => match mode {
                            NavMode::Header => StepAction::Descend(index),
                            NavMode::Dotted => {
                                StepAction::Reject(ParseDiagnosticKind::CannotExtendValue)
                            }
                        },
                        DefNode::Leaf(value) => {
                            // A leaf table value can only come from an inline
                            // literal, which is sealed at definition.
                            StepAction::Reject(match value {
                                Value::Table(_) => ParseDiagnosticKind::CannotExtendInlineTable,
                                _ => ParseDiagnosticKind::CannotExtendValue,
                            })
                        }
                    },
                    None => StepAction::Create(match mode {
                        NavMode::Dotted => Origin::Dotted,
                        NavMode::Header => Origin::HeaderImplicit,
                    }),
                }
            };
            match action {
                StepAction::Descend(index) => path.push(index),
                StepAction::Create(origin) => {
                    let slot = slot_at_mut(base, &path);
                    slot.entries.push(DefEntry {
                        path: segments[..=depth].to_vec(),
                        name,
                        key_span: segments[depth].span(),
                        node: DefNode::Table(TableSlot {
                            origin,
                            header: None,
                            key_span: segments[depth].span(),
                            entries: Vec::new(),
                        }),
                    });
                    let index = slot.entries.len() - 1;
                    path.push(index);
                }
                StepAction::Reject(kind) => {
                    self.problem(kind, segments[depth].span());
                    return None;
                }
            }
        }
        Some(path)
    }

    fn parse_key(&mut self) -> Option<Vec<KeySegment<'source>>> {
        let mut segments: Vec<KeySegment<'source>> = Vec::new();
        loop {
            let Some(token) = self.current() else {
                self.problem_at_cursor(ParseDiagnosticKind::ExpectedKey);
                return None;
            };
            match token.kind {
                SyntaxKind::BareKey => {
                    self.bump();
                    let raw = &self.source[token.span.range()];
                    let segment = KeySegment::new(raw, Some(Cow::Borrowed(raw)), token.span);
                    self.accepted_key_spans.push(segment.span());
                    segments.push(segment);
                }
                SyntaxKind::BasicString | SyntaxKind::LiteralString => {
                    self.bump();
                    let segment = self.quoted_key_segment(token)?;
                    self.accepted_key_spans.push(segment.span());
                    segments.push(segment);
                }
                kind if kind.can_start_key() => {
                    self.bump();
                    let reinterpreted = self.reinterpreted_key_segments(token)?;
                    for segment in reinterpreted {
                        self.accepted_key_spans.push(segment.span());
                        segments.push(segment);
                    }
                }
                _ => {
                    self.problem(ParseDiagnosticKind::ExpectedKey, token.span);
                    return None;
                }
            }
            if !self.consume(SyntaxKind::Dot) {
                return Some(segments);
            }
        }
    }

    fn quoted_key_segment(&mut self, token: LexToken) -> Option<KeySegment<'source>> {
        let raw = &self.source[token.span.range()];
        if token.has_error() {
            self.problem(ParseDiagnosticKind::InvalidKey, token.span);
            return None;
        }
        let decoded = match token.kind {
            SyntaxKind::LiteralString => Some(Cow::Borrowed(&raw[1..raw.len() - 1])),
            _ => match decode_basic(raw, token.span.start) {
                Ok(decoded) => Some(decoded),
                Err(DecodeError::NotScalar(span)) => {
                    self.problem(ParseDiagnosticKind::InvalidUnicodeScalar, span);
                    None
                }
                Err(DecodeError::Invalid) => {
                    self.problem(ParseDiagnosticKind::InvalidKey, token.span);
                    None
                }
            },
        };
        decoded.map(|decoded| KeySegment::new(raw, Some(decoded), token.span))
    }

    /// Accepts number, boolean, and date-time tokens as bare-key segments when
    /// their spelling fits the bare-key charset, splitting floats at dots.
    fn reinterpreted_key_segments(&mut self, token: LexToken) -> Option<Vec<KeySegment<'source>>> {
        let raw = &self.source[token.span.range()];
        let mut segments = Vec::new();
        let mut offset = token.span.start;
        for part in raw.split('.') {
            let span = Span::new(offset, offset + part.len());
            offset += part.len() + 1;
            if part.is_empty() || !part.bytes().all(is_bare_key_byte) {
                // The whole token was consumed as a key attempt; its lexical
                // diagnostics are covered by the parse-level invalid-key.
                self.accepted_key_spans.push(token.span);
                self.problem(ParseDiagnosticKind::InvalidKey, token.span);
                return None;
            }
            segments.push(KeySegment::new(part, Some(Cow::Borrowed(part)), span));
        }
        Some(segments)
    }

    fn parse_value(&mut self, depth: usize) -> Option<Value<'source>> {
        let Some(token) = self.current() else {
            self.problem_at_cursor(ParseDiagnosticKind::ExpectedValue);
            return None;
        };
        match token.kind {
            SyntaxKind::LeftBracket => {
                if depth >= self.options.max_depth {
                    self.problem(ParseDiagnosticKind::NestingLimitExceeded, token.span);
                    self.skip_bracketed(SyntaxKind::LeftBracket, SyntaxKind::RightBracket);
                    None
                } else {
                    Some(Value::Array(self.parse_array(depth + 1)))
                }
            }
            SyntaxKind::LeftBrace => {
                if depth >= self.options.max_depth {
                    self.problem(ParseDiagnosticKind::NestingLimitExceeded, token.span);
                    self.skip_bracketed(SyntaxKind::LeftBrace, SyntaxKind::RightBrace);
                    None
                } else {
                    Some(Value::Table(self.parse_inline_table(depth + 1)))
                }
            }
            SyntaxKind::BasicString
            | SyntaxKind::LiteralString
            | SyntaxKind::MultiLineBasicString
            | SyntaxKind::MultiLineLiteralString => {
                self.bump();
                Some(Value::String(self.string_value(token)))
            }
            SyntaxKind::Integer
            | SyntaxKind::HexInteger
            | SyntaxKind::OctInteger
            | SyntaxKind::BinInteger => {
                self.bump();
                Some(Value::Integer(self.integer_value(token)))
            }
            SyntaxKind::Float | SyntaxKind::Inf | SyntaxKind::Nan => {
                self.bump();
                Some(Value::Float(self.float_value(token)))
            }
            SyntaxKind::True => {
                self.bump();
                Some(Value::Boolean(Boolean::new(true, token.span)))
            }
            SyntaxKind::False => {
                self.bump();
                Some(Value::Boolean(Boolean::new(false, token.span)))
            }
            SyntaxKind::OffsetDateTime
            | SyntaxKind::LocalDateTime
            | SyntaxKind::LocalDate
            | SyntaxKind::LocalTime => {
                self.bump();
                let kind = match token.kind {
                    SyntaxKind::OffsetDateTime => DateTimeKind::OffsetDateTime,
                    SyntaxKind::LocalDateTime => DateTimeKind::LocalDateTime,
                    SyntaxKind::LocalDate => DateTimeKind::LocalDate,
                    _ => DateTimeKind::LocalTime,
                };
                Some(Value::DateTime(DateTime::new(
                    &self.source[token.span.range()],
                    token.span,
                    !token.has_error(),
                    kind,
                )))
            }
            _ => {
                self.problem(ParseDiagnosticKind::ExpectedValue, token.span);
                // Leave a line terminator in place so recovery keeps the next line.
                if token.kind != SyntaxKind::Newline {
                    self.bump();
                }
                None
            }
        }
    }

    fn integer_value(&self, token: LexToken) -> Integer<'source> {
        Integer::new(
            &self.source[token.span.range()],
            token.span,
            !token.has_error(),
        )
    }

    fn float_value(&self, token: LexToken) -> Float<'source> {
        Float::new(
            &self.source[token.span.range()],
            token.span,
            !token.has_error(),
        )
    }

    fn string_value(&mut self, token: LexToken) -> StringValue<'source> {
        let raw = &self.source[token.span.range()];
        let kind = match token.kind {
            SyntaxKind::BasicString => StringKind::Basic,
            SyntaxKind::LiteralString => StringKind::Literal,
            SyntaxKind::MultiLineBasicString => StringKind::MultiLineBasic,
            _ => StringKind::MultiLineLiteral,
        };
        if token.has_error() {
            return StringValue::new(raw, None, token.span, kind);
        }
        let decoded = match kind {
            StringKind::Basic => match decode_basic(raw, token.span.start) {
                Ok(decoded) => Some(decoded),
                Err(DecodeError::NotScalar(span)) => {
                    self.problem(ParseDiagnosticKind::InvalidUnicodeScalar, span);
                    None
                }
                Err(DecodeError::Invalid) => None,
            },
            StringKind::Literal => Some(Cow::Borrowed(&raw[1..raw.len() - 1])),
            StringKind::MultiLineBasic => match decode_multi_line_basic(raw, token.span.start) {
                Ok(decoded) => Some(decoded),
                Err(DecodeError::NotScalar(span)) => {
                    self.problem(ParseDiagnosticKind::InvalidUnicodeScalar, span);
                    None
                }
                Err(DecodeError::Invalid) => None,
            },
            StringKind::MultiLineLiteral => {
                Some(Cow::Borrowed(strip_first_newline(&raw[3..raw.len() - 3])))
            }
        };
        StringValue::new(raw, decoded, token.span, kind)
    }

    fn parse_array(&mut self, depth: usize) -> Array<'source> {
        let open = self.bump().expect("parse_array starts at `[` token");
        let mut elements = Vec::new();
        let mut end = open.span.end;

        loop {
            self.skip_line_breaks();
            let Some(token) = self.current() else {
                self.problem_at_cursor(ParseDiagnosticKind::ExpectedArrayEnd);
                break;
            };
            if token.kind == SyntaxKind::RightBracket {
                let close = self.bump().expect("current token exists");
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

            loop {
                self.skip_line_breaks();
                let Some(next) = self.current() else {
                    self.problem_at_cursor(ParseDiagnosticKind::ExpectedArrayEnd);
                    return Array::new(elements, Span::new(open.span.start, end));
                };
                match next.kind {
                    SyntaxKind::Comma => {
                        end = self.bump().expect("current token exists").span.end;
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

    fn parse_inline_table(&mut self, depth: usize) -> Table<'source> {
        let open = self.bump().expect("parse_inline_table starts at `{` token");
        let mut slot = TableSlot {
            origin: Origin::Inline,
            header: None,
            key_span: open.span,
            entries: Vec::new(),
        };
        loop {
            let Some(token) = self.current() else {
                self.problem_at_cursor(ParseDiagnosticKind::ExpectedInlineTableEnd);
                break;
            };
            match token.kind {
                SyntaxKind::RightBrace => {
                    let close = self.bump().expect("current token exists");
                    slot.key_span = Span::new(open.span.start, close.span.end);
                    break;
                }
                SyntaxKind::Newline => {
                    self.problem(ParseDiagnosticKind::ExpectedInlineTableEnd, token.span);
                    // Skip the offending break so the closing brace still seals
                    // the table and following entries keep parsing.
                    self.bump();
                    continue;
                }
                _ => {}
            }
            let Some(segments) = self.parse_key() else {
                self.recover_inline_table();
                continue;
            };
            if !self.consume(SyntaxKind::Equals) {
                self.problem_at_cursor(ParseDiagnosticKind::ExpectedEquals);
                self.recover_inline_table();
                continue;
            }
            let Some(value) = self.parse_value(depth) else {
                self.recover_inline_table();
                continue;
            };
            let last = segments
                .last()
                .expect("parse_key returns at least one segment");
            let key_span = last.span();
            let name = segment_name(last);
            let nav_path = self.navigate_intermediates(&mut slot, &segments, NavMode::Dotted);
            if let Some(nav_path) = nav_path {
                let duplicate = slot_at(&slot, &nav_path)
                    .entries
                    .iter()
                    .any(|entry| entry.name == name);
                if duplicate {
                    self.problem(ParseDiagnosticKind::DuplicateKey, key_span);
                } else {
                    let target = slot_at_mut(&mut slot, &nav_path);
                    target.entries.push(DefEntry {
                        path: segments,
                        name,
                        key_span,
                        node: DefNode::Leaf(value),
                    });
                }
            }
            self.inline_table_separator();
        }
        let key_span = slot.key_span;
        convert_slot_with(slot, key_span)
    }

    fn inline_table_separator(&mut self) {
        loop {
            let Some(next) = self.current() else {
                return;
            };
            match next.kind {
                SyntaxKind::Comma => {
                    self.bump();
                    if let Some(brace) = self.current()
                        && brace.kind == SyntaxKind::RightBrace
                    {
                        self.problem(ParseDiagnosticKind::ExpectedKey, brace.span);
                    }
                    return;
                }
                SyntaxKind::RightBrace | SyntaxKind::Newline => return,
                kind if kind.can_start_value() => {
                    self.problem(
                        ParseDiagnosticKind::ExpectedCommaOrEnd,
                        Span::new(next.span.start, next.span.start),
                    );
                    return;
                }
                _ => {
                    self.problem(ParseDiagnosticKind::ExpectedCommaOrEnd, next.span);
                    self.bump();
                }
            }
        }
    }

    fn recover_inline_table(&mut self) {
        while let Some(token) = self.current() {
            match token.kind {
                SyntaxKind::Comma => {
                    self.bump();
                    return;
                }
                SyntaxKind::RightBrace | SyntaxKind::Newline => return,
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn skip_bracketed(&mut self, open: SyntaxKind, close: SyntaxKind) {
        let mut depth = 0_usize;
        while let Some(token) = self.current() {
            self.bump();
            if token.kind == open {
                depth += 1;
            } else if token.kind == close {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
        }
    }

    fn skip_line_breaks(&mut self) {
        while self
            .current()
            .is_some_and(|token| token.kind == SyntaxKind::Newline)
        {
            self.bump();
        }
    }

    fn current(&self) -> Option<LexToken> {
        let mut cursor = self.cursor;
        while let Some(token) = self.tokens.get(cursor) {
            if token.kind.is_trivia() {
                cursor += 1;
            } else {
                return Some(*token);
            }
        }
        None
    }

    fn bump(&mut self) -> Option<LexToken> {
        while let Some(token) = self.tokens.get(self.cursor) {
            self.cursor += 1;
            if !token.kind.is_trivia() {
                self.last_end = token.span.end;
                return Some(*token);
            }
        }
        None
    }

    fn consume(&mut self, kind: SyntaxKind) -> bool {
        if self.current().is_some_and(|token| token.kind == kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn recover_to_line_end(&mut self) {
        while let Some(token) = self.current() {
            let is_newline = token.kind == SyntaxKind::Newline;
            self.bump();
            if is_newline {
                return;
            }
        }
    }

    fn expect_line_end(&mut self) {
        let Some(token) = self.current() else {
            return;
        };
        if token.kind == SyntaxKind::Newline {
            self.bump();
        } else {
            self.problem(ParseDiagnosticKind::ExpectedLineEnd, token.span);
            self.recover_to_line_end();
        }
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
        ParseDiagnosticKind::Lexical(_) | ParseDiagnosticKind::InvalidUnicodeScalar => 0,
        ParseDiagnosticKind::TooManyDiagnostics => 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::NumberError;
    use crate::ast::ValueKind;

    fn codes(source: &str) -> Vec<&'static str> {
        parse(source)
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.kind.code())
            .collect()
    }

    #[test]
    fn headers_and_dotted_keys_build_nested_tables() {
        let source =
            "name = \"tokenizer\"\n[package]\nauthors = [\"a\"]\n[package.meta]\nversion = 2\n";
        let document = parse(source).into_document();
        let root_table = document.root();
        assert_eq!(document.span(), Span::new(0, source.len()));
        assert_eq!(
            root_table.get("name").and_then(Value::as_str),
            Some("tokenizer")
        );
        let package = root_table
            .get("package")
            .and_then(Value::as_table)
            .expect("package table");
        assert_eq!(package.origin(), TableOrigin::Header);
        assert_eq!(package.len(), 2);
        let meta = package
            .get("meta")
            .and_then(Value::as_table)
            .expect("meta table");
        assert_eq!(
            meta.get("version")
                .and_then(Value::as_integer)
                .map(|value| value.as_i64()),
            Some(Ok(2))
        );
    }

    #[test]
    fn array_of_tables_collects_repeated_elements() {
        let source = "[[fruit]]\nname = \"apple\"\n[[fruit]]\nname = \"banana\"\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let fruit = parsed
            .document()
            .root()
            .get("fruit")
            .and_then(Value::as_array_of_tables)
            .expect("array of tables");
        assert_eq!(fruit.len(), 2);
        assert_eq!(
            fruit.elements()[0].get("name").and_then(Value::as_str),
            Some("apple")
        );
        assert_eq!(
            fruit.elements()[1].get("name").and_then(Value::as_str),
            Some("banana")
        );
    }

    #[test]
    fn dotted_keys_after_array_target_the_last_element() {
        let source = concat!(
            "[[fruit]]\n",
            "name = \"apple\"\n",
            "[[fruit.variety]]\n",
            "name = \"red delicious\"\n",
            "[[fruit.variety]]\n",
            "name = \"granny smith\"\n",
            "[[fruit]]\n",
            "name = \"banana\"\n",
            "[[fruit.variety]]\n",
            "name = \"plantain\"\n",
        );
        let parsed = parse(source);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        let fruit = parsed
            .document()
            .root()
            .get("fruit")
            .and_then(Value::as_array_of_tables)
            .expect("array of tables");
        fn varieties<'a>(table: &'a Table<'a>) -> Vec<Option<&'a str>> {
            table
                .get("variety")
                .and_then(Value::as_array_of_tables)
                .expect("variety array")
                .elements()
                .iter()
                .map(|element| element.get("name").and_then(Value::as_str))
                .collect()
        }
        assert_eq!(
            varieties(&fruit.elements()[0]),
            vec![Some("red delicious"), Some("granny smith")]
        );
        assert_eq!(varieties(&fruit.elements()[1]), vec![Some("plantain")]);
    }

    #[test]
    fn redefinition_rules_match_the_spec() {
        for (source, expected) in [
            ("[a]\n[a]\n", vec!["duplicate-key"]),
            ("[a.b]\n[a]\n", vec![]),
            ("a = 1\n[a]\n", vec!["cannot-extend-value"]),
            ("a = 1\na.b = 2\n", vec!["cannot-extend-value"]),
            ("a = {x = 1}\n[a.b]\n", vec!["cannot-extend-inline-table"]),
            ("a = {x = 1}\na.y = 2\n", vec!["cannot-extend-inline-table"]),
            (
                "[fruit]\napple.color = \"red\"\n[fruit.apple]\n",
                vec!["duplicate-key"],
            ),
            (
                "[fruit]\napple.color = \"red\"\n[fruit.apple.texture]\nsmooth = true\n",
                vec![],
            ),
            ("[[a]]\n[a]\n", vec!["duplicate-key"]),
            ("[[a]]\n[[a]]\n", vec![]),
            ("[x.y.z.w]\n[x]\n", vec![]),
            (
                "[a]\nb = 1\n[a]\nb = 2\n",
                vec!["duplicate-key", "duplicate-key"],
            ),
        ] {
            assert_eq!(codes(source), expected, "source: {source:?}");
        }
    }

    #[test]
    fn dotted_keys_create_nested_entries() {
        let source = "fruit.apple.color = \"red\"\nfruit.apple.taste.sweet = true\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let root_table = parsed.document().root();
        let fruit = root_table
            .get("fruit")
            .and_then(Value::as_table)
            .expect("fruit");
        assert_eq!(fruit.origin(), TableOrigin::Dotted);
        let apple = fruit.get("apple").and_then(Value::as_table).expect("apple");
        assert_eq!(apple.get("color").and_then(Value::as_str), Some("red"));
        let taste = apple.get("taste").and_then(Value::as_table).expect("taste");
        assert_eq!(taste.get("sweet").and_then(Value::as_bool), Some(true));
        assert_eq!(root_table.entries().len(), 1);
        let color = &apple.entries()[0];
        assert_eq!(color.key().segments().len(), 3);
        assert_eq!(color.span(), Span::new(0, 25));
        let sweet = &taste.entries()[0];
        assert_eq!(sweet.key().segments().len(), 4);
        assert_eq!(sweet.span(), Span::new(26, 56));
    }

    #[test]
    fn numeric_and_quoted_keys_are_reinterpreted() {
        for source in [
            "01 = 1\n",
            "1.2.3 = 1\n",
            "-0x10 = 1\n",
            "true = 1\n",
            "\"a b\" = 1\n",
        ] {
            let parsed = parse(source);
            assert!(
                parsed.is_valid(),
                "source: {source:?}, {:?}",
                parsed.diagnostics()
            );
        }
        assert_eq!(codes("12:00 = 1\n"), vec!["invalid-key"]);
        assert_eq!(codes("a = 1\n\"a\" = 2\n"), vec!["duplicate-key"]);
        assert_eq!(codes("\"\\uD800\" = 1\n"), vec!["invalid-unicode-scalar"]);
    }

    #[test]
    fn reinterpreted_keys_filter_lexical_diagnostics() {
        let parsed = parse("01 = 1\n2024-13-45 = 2\n");
        assert!(
            parsed.is_valid(),
            "lexical diagnostics leaked from keys: {:?}",
            parsed.diagnostics()
        );
        assert_eq!(
            parsed
                .document()
                .root()
                .get("01")
                .and_then(Value::as_integer)
                .map(|value| value.as_i64()),
            Some(Ok(1))
        );
        assert!(parsed.document().root().get("2024-13-45").is_some());
    }

    #[test]
    fn arrays_accept_trailing_commas_and_newlines() {
        for source in ["a = [\n  1,\n  2, 3,\n]\n", "a = [1, 2,]\n", "a = []\n"] {
            let parsed = parse(source);
            assert!(
                parsed.is_valid(),
                "source: {source:?}, {:?}",
                parsed.diagnostics()
            );
        }
        assert_eq!(codes("a = [,]\n"), vec!["expected-value"]);
        let sparse = parse("a = [1 2]\n");
        assert_eq!(
            sparse
                .diagnostics()
                .iter()
                .map(|d| d.kind.code())
                .collect::<Vec<_>>(),
            vec!["expected-comma-or-end"]
        );
        assert_eq!(
            sparse
                .document()
                .root()
                .get("a")
                .and_then(Value::as_array)
                .expect("array")
                .len(),
            2
        );
    }

    #[test]
    fn inline_tables_reject_trailing_commas_and_newlines() {
        assert_eq!(codes("a = {x = 1,}\n"), vec!["expected-key"]);
        assert_eq!(codes("a = {x = 1\n}\n"), vec!["expected-inline-table-end"]);
        for source in [
            "a = {x = 1, y = [2, 3]}\n",
            "a = {x = [\n1,\n]}\n",
            "a = {b.c = 1, b.d = 2}\n",
        ] {
            let parsed = parse(source);
            assert!(
                parsed.is_valid(),
                "source: {source:?}, {:?}",
                parsed.diagnostics()
            );
        }
        assert_eq!(
            codes("a = {x = 1}\n[a.y]\nz = 1\n"),
            vec!["cannot-extend-inline-table"]
        );
        let dotted = parse("a = {b.c = 1, b.d = 2}\n");
        let inner = dotted
            .document()
            .root()
            .get("a")
            .and_then(Value::as_table)
            .expect("inline table");
        assert_eq!(inner.origin(), TableOrigin::Inline);
        let b = inner.get("b").and_then(Value::as_table).expect("sub-table");
        assert_eq!(b.origin(), TableOrigin::Dotted);
    }

    #[test]
    fn all_four_string_forms_decode() {
        let parsed = parse(concat!(
            "a = \"x\\ty\"\n",
            "b = 'x\\ty'\n",
            "c = \"\"\"\nline\\\n  joined\"\"\"\n",
            "d = '''\nx\ny'''\n",
        ));
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        let root_table = parsed.document().root();
        assert_eq!(root_table.get("a").and_then(Value::as_str), Some("x\ty"));
        assert_eq!(root_table.get("b").and_then(Value::as_str), Some("x\\ty"));
        assert_eq!(
            root_table.get("c").and_then(Value::as_str),
            Some("linejoined")
        );
        assert_eq!(root_table.get("d").and_then(Value::as_str), Some("x\ny"));

        let broken = parse("a = \"\\uD800\"\n");
        assert_eq!(codes("a = \"\\uD800\"\n"), vec!["invalid-unicode-scalar"]);
        let value = broken.document().root().get("a").and_then(Value::as_str);
        assert!(value.is_none());
        assert!(matches!(
            broken.document().root().get("a"),
            Some(Value::String(string)) if !string.is_valid()
        ));
    }

    #[test]
    fn numbers_and_datetimes_stay_lossless() {
        let source = concat!(
            "a = 0xDE_AD\n",
            "b = -1_000\n",
            "c = 3.14\n",
            "d = inf\n",
            "e = nan\n",
            "f = 1979-05-27T07:32:00Z\n",
            "g = 1979-05-27\n",
            "h = 07:32:00\n",
            "i = 1979-05-27 07:32:00\n",
        );
        let parsed = parse(source);
        assert!(parsed.is_valid(), "{:?}", parsed.diagnostics());
        let root_table = parsed.document().root();
        let a = root_table
            .get("a")
            .and_then(Value::as_integer)
            .expect("integer");
        assert_eq!(a.as_i64(), Ok(0xDEAD));
        assert_eq!(a.as_f64(), Ok(57_005.0));
        let b = root_table
            .get("b")
            .and_then(Value::as_integer)
            .expect("integer");
        assert_eq!(b.as_i64(), Ok(-1_000));
        let e = root_table
            .get("e")
            .and_then(Value::as_float)
            .expect("float");
        assert!(e.as_f64().is_ok_and(|value| value.is_nan()));
        let kinds = [
            ("f", DateTimeKind::OffsetDateTime),
            ("g", DateTimeKind::LocalDate),
            ("h", DateTimeKind::LocalTime),
            ("i", DateTimeKind::LocalDateTime),
        ];
        for (key, kind) in kinds {
            let value = root_table
                .get(key)
                .and_then(Value::as_date_time)
                .expect(key);
            assert_eq!(value.kind(), kind, "key: {key}");
            assert!(value.is_valid(), "key: {key}");
        }

        let invalid = parse("x = 01\n");
        assert_eq!(codes("x = 01\n"), vec!["number-leading-zero"]);
        assert_eq!(
            invalid
                .document()
                .root()
                .get("x")
                .and_then(Value::as_integer)
                .map(|value| value.as_i64()),
            Some(Err(NumberError::InvalidTomlNumber))
        );
    }

    #[test]
    fn recovery_preserves_following_expressions() {
        let parsed = parse("a = ]\nb = 2\nc = 3\n");
        assert_eq!(
            parsed
                .diagnostics()
                .iter()
                .map(|d| d.kind.code())
                .collect::<Vec<_>>(),
            vec!["expected-value"]
        );
        let root_table = parsed.document().root();
        assert_eq!(
            root_table
                .get("b")
                .and_then(Value::as_integer)
                .map(|value| value.as_i64()),
            Some(Ok(2))
        );
        assert_eq!(
            root_table
                .get("c")
                .and_then(Value::as_integer)
                .map(|value| value.as_i64()),
            Some(Ok(3))
        );

        let missing = parse("a =\nb = 2\n");
        assert_eq!(codes("a =\nb = 2\n"), vec!["expected-value"]);
        assert!(missing.document().root().get("b").is_some());
    }

    #[test]
    fn nesting_and_diagnostic_limits_hold() {
        let deep = format!("a = {}1{}\n", "[".repeat(129), "]".repeat(129));
        let parsed = parse(&deep);
        assert_eq!(
            parsed
                .diagnostics()
                .iter()
                .map(|d| d.kind.code())
                .collect::<Vec<_>>(),
            vec!["nesting-limit-exceeded"]
        );
        assert_eq!(
            parse_with("a = [[[1]]]\n", ParseOptions::new().max_depth(2))
                .diagnostics()
                .iter()
                .map(|d| d.kind.code())
                .collect::<Vec<_>>(),
            vec!["nesting-limit-exceeded"]
        );

        let noisy = "a\na\na\na\na\n";
        let limited = parse_with(noisy, ParseOptions::new().max_diagnostics(2));
        let limited_codes = limited
            .diagnostics()
            .iter()
            .map(|d| d.kind.code())
            .collect::<Vec<_>>();
        assert_eq!(
            limited_codes,
            vec!["expected-equals", "expected-equals", "too-many-diagnostics"]
        );
        assert_eq!(
            parse_with(noisy, ParseOptions::new().max_diagnostics(500))
                .diagnostics()
                .len(),
            5
        );
    }

    #[test]
    fn empty_and_bom_documents_are_valid() {
        assert!(parse("").is_valid());
        assert!(parse("\n\n# comment only\n").is_valid());
        let bom = parse("\u{feff}a = 1\n");
        assert!(bom.is_valid(), "{:?}", bom.diagnostics());
        assert_eq!(
            bom.document()
                .root()
                .get("a")
                .and_then(Value::as_integer)
                .map(|value| value.as_i64()),
            Some(Ok(1))
        );
    }

    #[test]
    fn value_kinds_and_entry_spans_are_exact() {
        let source = "a = 1\nb = \"x\"\n";
        let parsed = parse(source);
        let root_table = parsed.document().root();
        assert_eq!(root_table.entries()[0].value().kind(), ValueKind::Integer);
        assert_eq!(root_table.entries()[1].value().kind(), ValueKind::String);
        assert_eq!(root_table.entries()[0].span(), Span::new(0, 5));
        assert_eq!(root_table.entries()[1].span(), Span::new(6, 13));
        assert_eq!(root_table.span(), Span::new(0, source.len()));
        assert_eq!(parsed.document().span(), Span::new(0, source.len()));
    }
}
