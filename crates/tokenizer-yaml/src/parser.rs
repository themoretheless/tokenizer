//! A recovering YAML 1.2 parser that builds a borrowing AST.
//!
//! The parser consumes the lossless token stream from the lexer and rebuilds
//! block structure from token spans, so it never branches on raw characters
//! outside block scalar content. It recovers from errors by skipping to the
//! next line or structural boundary, preserving as much of the document as
//! possible. All loops consume at least one token per iteration or break, so
//! malformed input terminates in linear time.

use std::borrow::Cow;
use std::fmt;

use crate::Span;
use crate::ast::{
    Alias, Anchor, CollectionStyle, Directive, Document, Entry, Mapping, Node, Scalar, ScalarStyle,
    Sequence, TagHandle,
};
use crate::lexer::{LexDiagnosticKind, LexToken, Lexed, LexerOptions, SyntaxKind};

/// Maximum nesting depth for recursive structures.
pub const MAX_SUPPORTED_DEPTH: usize = 128;

/// Options for parsing YAML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParseOptions {
    max_depth: usize,
    max_diagnostics: usize,
    lexer_options: LexerOptions,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl ParseOptions {
    /// Creates a new `ParseOptions` with default values.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_depth: MAX_SUPPORTED_DEPTH,
            max_diagnostics: 128,
            lexer_options: LexerOptions::new(),
        }
    }

    /// Sets the maximum nesting depth.
    #[must_use]
    pub const fn max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = if max_depth < MAX_SUPPORTED_DEPTH {
            max_depth
        } else {
            MAX_SUPPORTED_DEPTH
        };
        self
    }

    /// Sets the maximum number of diagnostics.
    #[must_use]
    pub const fn max_diagnostics(mut self, max_diagnostics: usize) -> Self {
        self.max_diagnostics = max_diagnostics;
        self
    }

    /// Sets the maximum number of tokens.
    #[must_use]
    pub const fn max_tokens(mut self, max_tokens: usize) -> Self {
        self.lexer_options = self.lexer_options.max_tokens(max_tokens);
        self
    }

    /// Sets the maximum input size in bytes.
    #[must_use]
    pub const fn max_input_bytes(mut self, max_input_bytes: usize) -> Self {
        self.lexer_options = self.lexer_options.max_input_bytes(max_input_bytes);
        self
    }

    /// Returns the lexer options.
    #[must_use]
    pub const fn lexer_options(&self) -> &LexerOptions {
        &self.lexer_options
    }

    /// Returns the depth limit.
    #[must_use]
    pub const fn depth_limit(&self) -> usize {
        self.max_depth
    }

    /// Returns the diagnostic limit.
    #[must_use]
    pub const fn diagnostic_limit(&self) -> usize {
        self.max_diagnostics
    }
}

/// The result of parsing YAML.
#[derive(Debug, Clone)]
pub struct Parse<'source> {
    source: &'source str,
    documents: Vec<Document<'source>>,
    lexed: Lexed<'source>,
    diagnostics: Vec<ParseDiagnostic>,
}

impl<'source> Parse<'source> {
    /// Returns the original source string.
    #[must_use]
    pub fn source(&self) -> &'source str {
        self.source
    }

    /// Returns the parsed documents.
    #[must_use]
    pub fn documents(&self) -> &[Document<'source>] {
        &self.documents
    }

    /// Returns the first document, if any.
    #[must_use]
    pub fn document(&self) -> Option<&Document<'source>> {
        self.documents.first()
    }

    /// Returns the lossless token stream.
    #[must_use]
    pub fn lexed(&self) -> &Lexed<'source> {
        &self.lexed
    }

    /// Returns the parse diagnostics, including every lexical diagnostic.
    #[must_use]
    pub fn diagnostics(&self) -> &[ParseDiagnostic] {
        &self.diagnostics
    }

    /// Returns `true` if there are any errors.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty() || self.lexed.has_errors()
    }

    /// Returns `true` if the parse succeeded without errors.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        !self.has_errors()
    }
}

/// A parse diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub kind: ParseDiagnosticKind,
    pub span: Span,
}

impl fmt::Display for ParseDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl std::error::Error for ParseDiagnostic {}

/// The kind of parse diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParseDiagnosticKind {
    Lexical(LexDiagnosticKind),
    ExpectedValue,
    ExpectedMappingEnd,
    ExpectedSequenceEnd,
    ExpectedKey,
    InvalidKey,
    DuplicateKey,
    NestingLimitExceeded,
    InvalidAnchor,
    InvalidAlias,
    InvalidTag,
    InvalidDirective,
    TooManyDiagnostics,
}

impl ParseDiagnosticKind {
    /// Returns the diagnostic code.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            ParseDiagnosticKind::Lexical(kind) => kind.code(),
            ParseDiagnosticKind::ExpectedValue => "expected-value",
            ParseDiagnosticKind::ExpectedMappingEnd => "expected-mapping-end",
            ParseDiagnosticKind::ExpectedSequenceEnd => "expected-sequence-end",
            ParseDiagnosticKind::ExpectedKey => "expected-key",
            ParseDiagnosticKind::InvalidKey => "invalid-key",
            ParseDiagnosticKind::DuplicateKey => "duplicate-key",
            ParseDiagnosticKind::NestingLimitExceeded => "nesting-limit-exceeded",
            ParseDiagnosticKind::InvalidAnchor => "invalid-anchor",
            ParseDiagnosticKind::InvalidAlias => "invalid-alias",
            ParseDiagnosticKind::InvalidTag => "invalid-tag",
            ParseDiagnosticKind::InvalidDirective => "invalid-directive",
            ParseDiagnosticKind::TooManyDiagnostics => "too-many-parse-diagnostics",
        }
    }
}

impl fmt::Display for ParseDiagnosticKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseDiagnosticKind::Lexical(kind) => write!(f, "{kind}"),
            ParseDiagnosticKind::ExpectedValue => write!(f, "expected a YAML value"),
            ParseDiagnosticKind::ExpectedMappingEnd => write!(f, "expected end of mapping"),
            ParseDiagnosticKind::ExpectedSequenceEnd => write!(f, "expected end of sequence"),
            ParseDiagnosticKind::ExpectedKey => write!(f, "expected a YAML key"),
            ParseDiagnosticKind::InvalidKey => write!(f, "invalid YAML key"),
            ParseDiagnosticKind::DuplicateKey => write!(f, "duplicate YAML key"),
            ParseDiagnosticKind::NestingLimitExceeded => {
                write!(f, "YAML nesting depth limit exceeded")
            }
            ParseDiagnosticKind::InvalidAnchor => write!(f, "invalid YAML anchor"),
            ParseDiagnosticKind::InvalidAlias => write!(f, "invalid YAML alias"),
            ParseDiagnosticKind::InvalidTag => write!(f, "invalid YAML tag"),
            ParseDiagnosticKind::InvalidDirective => write!(f, "invalid YAML directive"),
            ParseDiagnosticKind::TooManyDiagnostics => {
                write!(f, "additional parse diagnostics were omitted")
            }
        }
    }
}

/// Parses YAML source into a recovering AST.
///
/// ```
/// use themoretheless_tokenizer_yaml::parse;
///
/// let parsed = parse("name: yaml\n");
/// assert!(parsed.is_valid());
/// assert_eq!(parsed.documents().len(), 1);
/// assert!(parsed.document().unwrap().root().is_some());
/// ```
#[must_use]
pub fn parse(source: &str) -> Parse<'_> {
    parse_with(source, ParseOptions::new())
}

/// Parses YAML source with custom options.
#[must_use]
pub fn parse_with(source: &str, options: ParseOptions) -> Parse<'_> {
    let lexed = crate::lexer::lex_with(source, *options.lexer_options());
    let mut parser = Parser::new(source, &lexed, options);
    let documents = parser.parse_stream();
    let diagnostics = parser.finish(&lexed);
    Parse {
        source,
        documents,
        lexed,
        diagnostics,
    }
}

struct Parser<'source> {
    source: &'source str,
    sig: Vec<LexToken>,
    cursor: usize,
    line_starts: Vec<usize>,
    diagnostics: Vec<ParseDiagnostic>,
    options: ParseOptions,
    depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chomp {
    Clip,
    Strip,
    Keep,
}

impl<'source> Parser<'source> {
    fn new(source: &'source str, lexed: &Lexed<'source>, options: ParseOptions) -> Self {
        let sig = lexed
            .tokens()
            .iter()
            .copied()
            .filter(|token| !token.kind.is_trivia())
            .collect();
        let mut line_starts = vec![0usize];
        for (offset, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(offset + 1);
            }
        }
        Self {
            source,
            sig,
            cursor: 0,
            line_starts,
            diagnostics: Vec::new(),
            options,
            depth: 0,
        }
    }

    fn finish(self, lexed: &Lexed<'source>) -> Vec<ParseDiagnostic> {
        let mut diagnostics = self.diagnostics;
        for diagnostic in lexed.diagnostics() {
            diagnostics.push(ParseDiagnostic {
                kind: ParseDiagnosticKind::Lexical(diagnostic.kind),
                span: diagnostic.span,
            });
        }
        diagnostics.sort_by_key(|diagnostic| {
            (
                diagnostic.span.start,
                diagnostic.span.end,
                diagnostic.kind.code(),
            )
        });
        diagnostics.dedup_by(|a, b| a.kind == b.kind && a.span == b.span);
        let limit = self.options.max_diagnostics;
        if diagnostics.len() > limit {
            diagnostics.truncate(limit);
            diagnostics.push(ParseDiagnostic {
                kind: ParseDiagnosticKind::TooManyDiagnostics,
                span: Span::new(self.source.len(), self.source.len()),
            });
        }
        diagnostics
    }

    // ─── Cursor helpers ──────────────────────────────────────────────────

    fn peek(&self) -> Option<LexToken> {
        self.sig.get(self.cursor).copied()
    }

    fn peek_is(&self, kind: SyntaxKind) -> bool {
        self.peek().is_some_and(|token| token.kind == kind)
    }

    fn at(&self, index: usize) -> Option<LexToken> {
        self.sig.get(index).copied()
    }

    fn advance(&mut self) {
        if self.cursor < self.sig.len() {
            self.cursor += 1;
        }
    }

    fn is_eof(&self) -> bool {
        self.cursor >= self.sig.len()
    }

    fn current_span(&self) -> Option<Span> {
        self.sig.get(self.cursor).map(|token| token.span)
    }

    fn current_col(&self) -> usize {
        self.current_span()
            .map_or(0, |span| self.column_of(span.start))
    }

    fn token_text(&self, token: LexToken) -> &'source str {
        token.text(self.source).unwrap_or_default()
    }

    fn line_index(&self, offset: usize) -> usize {
        self.line_starts.partition_point(|&start| start <= offset) - 1
    }

    fn column_of(&self, offset: usize) -> usize {
        let line_start = self.line_starts[self.line_index(offset)];
        let mut column = offset - line_start;
        if column > 0 && offset < self.source.len() && self.source.as_bytes()[offset - 1] == b'\r' {
            column -= 1;
        }
        column
    }

    fn same_line(&self, left: usize, right: usize) -> bool {
        self.line_index(left) == self.line_index(right)
    }

    fn skip_breaks(&mut self) {
        while self.peek_is(SyntaxKind::LineBreak) {
            self.advance();
        }
    }

    fn skip_line_end(&mut self) {
        if self.peek_is(SyntaxKind::LineBreak) {
            self.advance();
        }
    }

    fn skip_to_next_line(&mut self) {
        while let Some(token) = self.peek() {
            self.advance();
            if token.kind == SyntaxKind::LineBreak {
                break;
            }
        }
    }

    fn error(&mut self, kind: ParseDiagnosticKind) {
        let span = self
            .current_span()
            .unwrap_or_else(|| Span::new(self.source.len(), self.source.len()));
        self.diagnostics.push(ParseDiagnostic { kind, span });
    }

    fn error_at(&mut self, kind: ParseDiagnosticKind, span: Span) {
        self.diagnostics.push(ParseDiagnostic { kind, span });
    }

    fn empty_scalar(&self, offset: usize) -> Node<'source> {
        Node::Scalar(Scalar::new(
            "",
            Some(Cow::Borrowed("")),
            None,
            None,
            Span::new(offset, offset),
            ScalarStyle::Plain,
            true,
        ))
    }

    fn attach(
        &self,
        node: &mut Node<'source>,
        anchor: Option<Anchor<'source>>,
        tag: Option<TagHandle<'source>>,
    ) {
        match node {
            Node::Scalar(scalar) => {
                if anchor.is_some() {
                    scalar.anchor = anchor;
                }
                if tag.is_some() {
                    scalar.tag = tag;
                }
            }
            Node::Sequence(sequence) => {
                if anchor.is_some() {
                    sequence.anchor = anchor;
                }
                if tag.is_some() {
                    sequence.tag = tag;
                }
            }
            Node::Mapping(mapping) => {
                if anchor.is_some() {
                    mapping.anchor = anchor;
                }
                if tag.is_some() {
                    mapping.tag = tag;
                }
            }
            Node::Alias(_) => {}
        }
    }

    // ─── Stream and document structure ───────────────────────────────────

    fn parse_stream(&mut self) -> Vec<Document<'source>> {
        let mut documents = Vec::new();
        self.skip_breaks();
        while !self.is_eof() {
            let before = self.cursor;
            documents.push(self.parse_document());
            if self.cursor == before {
                self.advance();
            }
            self.skip_breaks();
        }
        if documents.is_empty() {
            documents.push(Document::new(Vec::new(), None, None, None, Span::new(0, 0)));
        }
        documents
    }

    fn parse_document(&mut self) -> Document<'source> {
        let start = self.current_span().map_or(0, |span| span.start);
        let mut directives = Vec::new();
        let mut start_marker = None;
        let mut end_marker = None;

        while let Some(token) = self.peek() {
            match token.kind {
                SyntaxKind::Directive => {
                    let raw = self.token_text(token);
                    directives.push(Directive::new(raw, token.span));
                    self.advance();
                    self.skip_line_end();
                }
                SyntaxKind::DocumentStart => {
                    start_marker = Some(token.span);
                    self.advance();
                    break;
                }
                _ => break,
            }
        }

        self.skip_breaks();
        let root = if self.is_eof()
            || self.peek_is(SyntaxKind::DocumentEnd)
            || self.peek_is(SyntaxKind::DocumentStart)
        {
            None
        } else {
            self.parse_node(None)
        };

        self.skip_breaks();
        if self.peek_is(SyntaxKind::DocumentEnd) {
            end_marker = self.current_span();
            self.advance();
        }
        while !self.is_eof()
            && !self.peek_is(SyntaxKind::DocumentStart)
            && !self.peek_is(SyntaxKind::DocumentEnd)
        {
            self.skip_to_next_line();
        }

        let end = self
            .current_span()
            .map_or(self.source.len(), |span| span.start);
        Document::new(
            directives,
            root,
            start_marker,
            end_marker,
            Span::new(start, end.max(start)),
        )
    }
}

impl<'source> Parser<'source> {
    // ─── Lookaheads ──────────────────────────────────────────────────────

    /// True when the token at the cursor is a scalar that introduces an
    /// implicit `key:` pair: a value indicator directly follows it on the
    /// same line.
    fn scalar_key_ahead(&self) -> bool {
        self.scalar_key_ahead_at(self.cursor)
    }

    fn scalar_key_ahead_at(&self, index: usize) -> bool {
        let Some(first) = self.at(index) else {
            return false;
        };
        if !matches!(
            first.kind,
            SyntaxKind::PlainScalar
                | SyntaxKind::SingleQuotedScalar
                | SyntaxKind::DoubleQuotedScalar
        ) {
            return false;
        }
        let Some(next) = self.at(index + 1) else {
            return false;
        };
        next.kind == SyntaxKind::ValueIndicator && self.same_line(next.span.start, first.span.end)
    }

    /// True when the flow collection starting at the cursor is followed
    /// directly by a value indicator, making it a key.
    fn flow_key_ahead(&self) -> bool {
        let Some(first) = self.peek() else {
            return false;
        };
        if !matches!(
            first.kind,
            SyntaxKind::FlowSequenceStart | SyntaxKind::FlowMappingStart
        ) {
            return false;
        }
        let mut depth = 0usize;
        let mut index = self.cursor;
        while index < self.sig.len() {
            match self.sig[index].kind {
                SyntaxKind::FlowSequenceStart | SyntaxKind::FlowMappingStart => depth += 1,
                SyntaxKind::FlowSequenceEnd | SyntaxKind::FlowMappingEnd => {
                    depth -= 1;
                    if depth == 0 {
                        return self
                            .at(index + 1)
                            .is_some_and(|token| token.kind == SyntaxKind::ValueIndicator);
                    }
                }
                _ => {}
            }
            index += 1;
        }
        false
    }

    // ─── Nodes ───────────────────────────────────────────────────────────

    /// Parses one block or flow node. `floor` is the column content must
    /// exceed; `None` accepts any column.
    fn parse_node(&mut self, floor: Option<usize>) -> Option<Node<'source>> {
        if self.depth >= self.options.max_depth {
            self.error(ParseDiagnosticKind::NestingLimitExceeded);
            return None;
        }
        self.skip_breaks();
        let token = self.peek()?;
        if matches!(
            token.kind,
            SyntaxKind::DocumentStart | SyntaxKind::DocumentEnd
        ) {
            return None;
        }
        let col = self.column_of(token.span.start);
        if let Some(floor) = floor
            && col <= floor
        {
            return None;
        }

        let mut anchor: Option<Anchor<'source>> = None;
        let mut tag: Option<TagHandle<'source>> = None;
        while let Some(token) = self.peek() {
            match token.kind {
                SyntaxKind::Anchor if anchor.is_none() => {
                    let text = self.token_text(token);
                    anchor = Some(Anchor::new(
                        text.strip_prefix('&').unwrap_or(text),
                        token.span,
                    ));
                }
                SyntaxKind::Tag if tag.is_none() => {
                    tag = Some(TagHandle::new(self.token_text(token), token.span));
                }
                _ => break,
            }
            self.advance();
        }

        let Some(token) = self.peek() else {
            if anchor.is_some() || tag.is_some() {
                let offset = self
                    .current_span()
                    .map_or(self.source.len(), |span| span.end);
                let mut node = self.empty_scalar(offset);
                self.attach(&mut node, anchor, tag);
                return Some(node);
            }
            return None;
        };

        self.depth += 1;
        let node = match token.kind {
            SyntaxKind::FlowSequenceStart => {
                if self.flow_key_ahead() {
                    Node::Mapping(self.parse_block_mapping(col))
                } else {
                    Node::Sequence(self.parse_flow_sequence())
                }
            }
            SyntaxKind::FlowMappingStart => {
                if self.flow_key_ahead() {
                    Node::Mapping(self.parse_block_mapping(col))
                } else {
                    Node::Mapping(self.parse_flow_mapping())
                }
            }
            SyntaxKind::BlockEntry => Node::Sequence(self.parse_block_sequence(col)),
            SyntaxKind::KeyIndicator | SyntaxKind::ValueIndicator => {
                Node::Mapping(self.parse_block_mapping(col))
            }
            SyntaxKind::BlockScalarHeader => {
                Node::Scalar(self.parse_block_scalar(floor, col, anchor.take(), tag.take()))
            }
            SyntaxKind::SingleQuotedScalar | SyntaxKind::DoubleQuotedScalar => {
                if self.scalar_key_ahead() {
                    Node::Mapping(self.parse_block_mapping(col))
                } else {
                    Node::Scalar(self.parse_quoted_scalar(anchor.take(), tag.take()))
                }
            }
            SyntaxKind::PlainScalar => {
                if self.scalar_key_ahead() {
                    Node::Mapping(self.parse_block_mapping(col))
                } else {
                    Node::Scalar(self.parse_plain_scalar(floor, anchor.take(), tag.take()))
                }
            }
            SyntaxKind::Alias => {
                let text = self.token_text(token);
                let alias = Alias::new(text.strip_prefix('*').unwrap_or(text), token.span);
                self.advance();
                Node::Alias(alias)
            }
            _ => {
                self.depth -= 1;
                return None;
            }
        };
        self.depth -= 1;

        let mut node = node;
        self.attach(&mut node, anchor, tag);
        Some(node)
    }

    // ─── Block mappings ──────────────────────────────────────────────────

    fn parse_block_mapping(&mut self, map_col: usize) -> Mapping<'source> {
        let start = self.current_span().map_or(0, |span| span.start);
        let mut entries = Vec::new();
        let mut end = start;
        let mut seen: Vec<&'source str> = Vec::new();

        loop {
            self.skip_breaks();
            let Some(token) = self.peek() else {
                break;
            };
            if matches!(
                token.kind,
                SyntaxKind::DocumentStart | SyntaxKind::DocumentEnd
            ) {
                break;
            }
            if self.column_of(token.span.start) != map_col {
                break;
            }
            let entry = match token.kind {
                SyntaxKind::KeyIndicator => self.parse_explicit_entry(map_col),
                SyntaxKind::ValueIndicator => self.parse_null_key_entry(map_col),
                SyntaxKind::PlainScalar
                | SyntaxKind::SingleQuotedScalar
                | SyntaxKind::DoubleQuotedScalar
                    if self.scalar_key_ahead() =>
                {
                    self.parse_implicit_entry(map_col)
                }
                SyntaxKind::FlowSequenceStart | SyntaxKind::FlowMappingStart
                    if self.flow_key_ahead() =>
                {
                    self.parse_implicit_entry(map_col)
                }
                _ => break,
            };
            let Some(entry) = entry else {
                break;
            };
            if let Some(key) = entry.key().as_scalar() {
                let text = key.raw();
                if seen.contains(&text) {
                    self.error_at(ParseDiagnosticKind::DuplicateKey, key.span());
                } else {
                    seen.push(text);
                }
            }
            end = entry.span().end;
            entries.push(entry);
        }

        Mapping::new(
            entries,
            None,
            None,
            Span::new(start, end),
            CollectionStyle::Block,
        )
    }

    fn parse_implicit_entry(&mut self, map_col: usize) -> Option<Entry<'source>> {
        let key = self.parse_key()?;
        let key_span = key.span();
        let Some(colon) = self.peek() else {
            return Some(Entry::new(key, None, key_span));
        };
        if colon.kind != SyntaxKind::ValueIndicator {
            self.error(ParseDiagnosticKind::ExpectedValue);
            return Some(Entry::new(key, None, key_span));
        }
        self.advance();
        let value = self.parse_value_after_colon(map_col, colon);
        let end = value
            .as_ref()
            .map_or(colon.span.end, |node| node.span().end);
        Some(Entry::new(key, value, Span::new(key_span.start, end)))
    }

    fn parse_null_key_entry(&mut self, map_col: usize) -> Option<Entry<'source>> {
        let colon = self.peek()?;
        self.advance();
        let key = self.empty_scalar(colon.span.start);
        let value = self.parse_value_after_colon(map_col, colon);
        let end = value
            .as_ref()
            .map_or(colon.span.end, |node| node.span().end);
        Some(Entry::new(key, value, Span::new(colon.span.start, end)))
    }

    fn parse_explicit_entry(&mut self, map_col: usize) -> Option<Entry<'source>> {
        let marker = self.peek()?;
        self.advance();
        let key = match self.parse_node(Some(map_col)) {
            Some(key) => key,
            None => {
                self.error(ParseDiagnosticKind::ExpectedKey);
                self.empty_scalar(marker.span.end)
            }
        };
        self.skip_breaks();
        let mut end = key.span().end;
        let value = if self.peek_is(SyntaxKind::ValueIndicator) && self.current_col() == map_col {
            let colon = self.peek()?;
            self.advance();
            end = colon.span.end;
            let value = self.parse_value_after_colon(map_col, colon);
            if let Some(value) = &value {
                end = value.span().end;
            }
            value
        } else {
            None
        };
        Some(Entry::new(
            key,
            value,
            Span::new(marker.span.start, end.max(marker.span.start)),
        ))
    }

    fn parse_value_after_colon(&mut self, indent: usize, colon: LexToken) -> Option<Node<'source>> {
        if let Some(token) = self.peek()
            && token.kind != SyntaxKind::LineBreak
            && self.same_line(token.span.start, colon.span.start)
        {
            return self.parse_node(Some(indent));
        }
        self.skip_breaks();
        let token = self.peek()?;
        let col = self.column_of(token.span.start);
        if col > indent {
            self.parse_node(Some(indent))
        } else if col == indent && token.kind == SyntaxKind::BlockEntry {
            Some(Node::Sequence(self.parse_block_sequence(col)))
        } else {
            None
        }
    }

    fn parse_key(&mut self) -> Option<Node<'source>> {
        let token = self.peek()?;
        match token.kind {
            SyntaxKind::PlainScalar => Some(Node::Scalar(self.parse_plain_token())),
            SyntaxKind::SingleQuotedScalar | SyntaxKind::DoubleQuotedScalar => {
                Some(Node::Scalar(self.parse_quoted_scalar(None, None)))
            }
            SyntaxKind::FlowSequenceStart => Some(Node::Sequence(self.parse_flow_sequence())),
            SyntaxKind::FlowMappingStart => Some(Node::Mapping(self.parse_flow_mapping())),
            _ => None,
        }
    }

    // ─── Block sequences ─────────────────────────────────────────────────

    fn parse_block_sequence(&mut self, dash_col: usize) -> Sequence<'source> {
        let start = self.current_span().map_or(0, |span| span.start);
        let mut elements = Vec::new();
        let mut end = start;

        while let Some(dash) = self.peek() {
            if dash.kind != SyntaxKind::BlockEntry || self.column_of(dash.span.start) != dash_col {
                break;
            }
            self.advance();
            end = dash.span.end;
            self.depth += 1;
            let value = self.parse_node(Some(dash_col));
            self.depth -= 1;
            let node = match value {
                Some(node) => {
                    end = node.span().end;
                    node
                }
                None => self.empty_scalar(dash.span.end),
            };
            elements.push(node);
        }

        Sequence::new(
            elements,
            None,
            None,
            Span::new(start, end),
            CollectionStyle::Block,
        )
    }
}

impl<'source> Parser<'source> {
    // ─── Flow collections ────────────────────────────────────────────────

    fn parse_flow_sequence(&mut self) -> Sequence<'source> {
        let start = self.current_span().map_or(0, |span| span.start);
        self.advance();
        let mut elements = Vec::new();

        loop {
            self.skip_breaks();
            match self.peek() {
                None => break,
                Some(token)
                    if matches!(
                        token.kind,
                        SyntaxKind::DocumentStart | SyntaxKind::DocumentEnd
                    ) =>
                {
                    break;
                }
                Some(token) if token.kind == SyntaxKind::FlowSequenceEnd => {
                    let end = token.span.end;
                    self.advance();
                    return Sequence::new(
                        elements,
                        None,
                        None,
                        Span::new(start, end),
                        CollectionStyle::Flow,
                    );
                }
                Some(token) if token.kind == SyntaxKind::FlowEntry => {
                    self.advance();
                    continue;
                }
                Some(_) => {}
            }
            let before = self.cursor;
            if let Some(node) = self.parse_flow_node() {
                elements.push(node);
            }
            if self.cursor == before {
                self.skip_flow_junk();
            }
        }

        self.error(ParseDiagnosticKind::ExpectedSequenceEnd);
        let end = self
            .current_span()
            .map_or(self.source.len(), |span| span.start);
        Sequence::new(
            elements,
            None,
            None,
            Span::new(start, end.max(start)),
            CollectionStyle::Flow,
        )
    }

    fn parse_flow_mapping(&mut self) -> Mapping<'source> {
        let start = self.current_span().map_or(0, |span| span.start);
        self.advance();
        let mut entries = Vec::new();

        loop {
            self.skip_breaks();
            match self.peek() {
                None => break,
                Some(token)
                    if matches!(
                        token.kind,
                        SyntaxKind::DocumentStart | SyntaxKind::DocumentEnd
                    ) =>
                {
                    break;
                }
                Some(token) if token.kind == SyntaxKind::FlowMappingEnd => {
                    let end = token.span.end;
                    self.advance();
                    return Mapping::new(
                        entries,
                        None,
                        None,
                        Span::new(start, end),
                        CollectionStyle::Flow,
                    );
                }
                Some(token) if token.kind == SyntaxKind::FlowEntry => {
                    self.advance();
                    continue;
                }
                Some(_) => {}
            }
            let before = self.cursor;
            if let Some(entry) = self.parse_flow_entry() {
                entries.push(entry);
            }
            if self.cursor == before {
                self.skip_flow_junk();
            }
        }

        self.error(ParseDiagnosticKind::ExpectedMappingEnd);
        let end = self
            .current_span()
            .map_or(self.source.len(), |span| span.start);
        Mapping::new(
            entries,
            None,
            None,
            Span::new(start, end.max(start)),
            CollectionStyle::Flow,
        )
    }

    fn parse_flow_entry(&mut self) -> Option<Entry<'source>> {
        let key = self.parse_flow_value()?;
        let key_span = key.span();
        self.skip_breaks();
        if self.peek_is(SyntaxKind::ValueIndicator) {
            let colon = self.peek()?;
            self.advance();
            self.skip_breaks();
            let value = if self.at(self.cursor).is_some_and(|token| {
                matches!(
                    token.kind,
                    SyntaxKind::FlowEntry
                        | SyntaxKind::FlowSequenceEnd
                        | SyntaxKind::FlowMappingEnd
                )
            }) || self.is_eof()
            {
                None
            } else {
                self.parse_flow_value()
            };
            let end = value
                .as_ref()
                .map_or(colon.span.end, |node| node.span().end);
            Some(Entry::new(key, value, Span::new(key_span.start, end)))
        } else {
            Some(Entry::new(key, None, key_span))
        }
    }

    /// A flow node, possibly a single-pair implicit mapping.
    fn parse_flow_node(&mut self) -> Option<Node<'source>> {
        let key = self.parse_flow_value()?;
        self.skip_breaks();
        if !self.peek_is(SyntaxKind::ValueIndicator) {
            return Some(key);
        }
        let key_span = key.span();
        let colon = self.peek()?;
        self.advance();
        self.skip_breaks();
        let value = if self.is_eof()
            || self.at(self.cursor).is_some_and(|token| {
                matches!(
                    token.kind,
                    SyntaxKind::FlowEntry
                        | SyntaxKind::FlowSequenceEnd
                        | SyntaxKind::FlowMappingEnd
                )
            }) {
            None
        } else {
            self.parse_flow_value()
        };
        let end = value
            .as_ref()
            .map_or(colon.span.end, |node| node.span().end);
        let entry = Entry::new(key, value, Span::new(key_span.start, end));
        Some(Node::Mapping(Mapping::new(
            vec![entry],
            None,
            None,
            Span::new(key_span.start, end),
            CollectionStyle::Flow,
        )))
    }

    fn parse_flow_value(&mut self) -> Option<Node<'source>> {
        if self.depth >= self.options.max_depth {
            self.error(ParseDiagnosticKind::NestingLimitExceeded);
            return None;
        }
        self.skip_breaks();
        let mut anchor: Option<Anchor<'source>> = None;
        let mut tag: Option<TagHandle<'source>> = None;
        while let Some(token) = self.peek() {
            match token.kind {
                SyntaxKind::Anchor if anchor.is_none() => {
                    let text = self.token_text(token);
                    anchor = Some(Anchor::new(
                        text.strip_prefix('&').unwrap_or(text),
                        token.span,
                    ));
                }
                SyntaxKind::Tag if tag.is_none() => {
                    tag = Some(TagHandle::new(self.token_text(token), token.span));
                }
                _ => break,
            }
            self.advance();
        }
        let token = self.peek()?;
        self.depth += 1;
        let mut node = match token.kind {
            SyntaxKind::FlowSequenceStart => Node::Sequence(self.parse_flow_sequence()),
            SyntaxKind::FlowMappingStart => Node::Mapping(self.parse_flow_mapping()),
            SyntaxKind::SingleQuotedScalar | SyntaxKind::DoubleQuotedScalar => {
                Node::Scalar(self.parse_quoted_scalar(None, None))
            }
            SyntaxKind::PlainScalar => Node::Scalar(self.parse_plain_token()),
            SyntaxKind::Alias => {
                let text = self.token_text(token);
                let alias = Alias::new(text.strip_prefix('*').unwrap_or(text), token.span);
                self.advance();
                Node::Alias(alias)
            }
            _ => {
                self.depth -= 1;
                return None;
            }
        };
        self.depth -= 1;
        self.attach(&mut node, anchor, tag);
        Some(node)
    }

    fn skip_flow_junk(&mut self) {
        while let Some(token) = self.peek() {
            if matches!(
                token.kind,
                SyntaxKind::FlowEntry
                    | SyntaxKind::FlowSequenceEnd
                    | SyntaxKind::FlowMappingEnd
                    | SyntaxKind::LineBreak
                    | SyntaxKind::DocumentStart
                    | SyntaxKind::DocumentEnd
            ) {
                return;
            }
            self.advance();
        }
    }

    // ─── Scalars ─────────────────────────────────────────────────────────

    /// A single plain scalar token without continuation lines.
    fn parse_plain_token(&mut self) -> Scalar<'source> {
        let token = self.peek().unwrap();
        let raw = self.token_text(token);
        self.advance();
        let valid = !token.has_error();
        Scalar::new(
            raw,
            valid.then_some(Cow::Borrowed(raw)),
            None,
            None,
            token.span,
            ScalarStyle::Plain,
            valid,
        )
    }

    /// A plain scalar, folded across continuation lines whose columns exceed
    /// the floor.
    fn parse_plain_scalar(
        &mut self,
        floor: Option<usize>,
        anchor: Option<Anchor<'source>>,
        tag: Option<TagHandle<'source>>,
    ) -> Scalar<'source> {
        let first = self.peek().unwrap();
        let start = first.span.start;
        let mut end = first.span.end;
        let mut pieces: Vec<&'source str> = vec![self.token_text(first)];
        let mut gaps: Vec<usize> = Vec::new();
        self.advance();
        let min_column = floor.map_or(1, |floor| floor + 1);

        loop {
            let mut breaks = 0usize;
            while self.peek_is(SyntaxKind::LineBreak) {
                breaks += 1;
                self.advance();
            }
            if breaks == 0 {
                break;
            }
            let Some(token) = self.peek() else {
                break;
            };
            if token.kind != SyntaxKind::PlainScalar {
                break;
            }
            if self.column_of(token.span.start) < min_column {
                break;
            }
            if self.scalar_key_ahead_at(self.cursor) {
                break;
            }
            gaps.push(breaks);
            pieces.push(self.token_text(token));
            end = token.span.end;
            self.advance();
        }

        let mut out = String::new();
        for (index, piece) in pieces.iter().enumerate() {
            if index > 0 {
                let gap = gaps[index - 1];
                if gap == 1 {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                } else {
                    out.push_str(&"\n".repeat(gap - 1));
                }
            }
            out.push_str(piece.trim_end_matches([' ', '\t']));
        }

        let raw = &self.source[start..end];
        Scalar::new(
            raw,
            Some(Cow::Owned(out)),
            anchor,
            tag,
            Span::new(start, end),
            ScalarStyle::Plain,
            true,
        )
    }

    fn parse_quoted_scalar(
        &mut self,
        anchor: Option<Anchor<'source>>,
        tag: Option<TagHandle<'source>>,
    ) -> Scalar<'source> {
        let token = self.peek().unwrap();
        let span = token.span;
        let raw = self.token_text(token);
        let style = if token.kind == SyntaxKind::SingleQuotedScalar {
            ScalarStyle::SingleQuoted
        } else {
            ScalarStyle::DoubleQuoted
        };
        self.advance();
        let decoded = if token.has_error() {
            None
        } else {
            match style {
                ScalarStyle::SingleQuoted => decode_single_quoted(raw),
                _ => decode_double_quoted(raw),
            }
        };
        let valid = !token.has_error() && decoded.is_some();
        Scalar::new(
            raw,
            decoded.map(Cow::Owned),
            anchor,
            tag,
            span,
            style,
            valid,
        )
    }

    fn parse_block_scalar(
        &mut self,
        floor: Option<usize>,
        header_col: usize,
        anchor: Option<Anchor<'source>>,
        tag: Option<TagHandle<'source>>,
    ) -> Scalar<'source> {
        let header = self.peek().unwrap();
        let header_span = header.span;
        self.advance();
        let header_text = self.token_text(header);
        let style = if header_text.starts_with('|') {
            ScalarStyle::Literal
        } else {
            ScalarStyle::Folded
        };
        let mut explicit_indent: Option<usize> = None;
        let mut chomp = Chomp::Clip;
        for c in header_text.chars().skip(1) {
            match c {
                '1'..='9' => explicit_indent = Some(c as usize - '0' as usize),
                '-' => chomp = Chomp::Strip,
                '+' => chomp = Chomp::Keep,
                _ => {}
            }
        }

        let content_indent = explicit_indent.map(|digits| header_col + digits);
        let mut lines: Vec<&'source str> = Vec::new();
        let mut raw_end = header_span.end;
        let mut last_terminated = false;
        let mut index = self.line_index(header_span.start) + 1;

        // Phase 1: find the indentation from the first non-blank line when it
        // was not given explicitly.
        let mut first_line = index;
        let mut pending_blank: Option<usize> = None;
        let mut content_indent = content_indent;
        while index < self.line_starts.len() {
            let (text, _, _) = self.line_at(index);
            if text.trim_start_matches(' ').is_empty() {
                pending_blank.get_or_insert(index);
                index += 1;
                continue;
            }
            let column = self.indent_of(text);
            match content_indent {
                Some(indent) => {
                    if column < indent {
                        return self.finish_block_scalar(
                            header_span,
                            style,
                            chomp,
                            lines,
                            raw_end,
                            last_terminated,
                            anchor,
                            tag,
                        );
                    }
                    first_line = pending_blank.unwrap_or(index);
                    break;
                }
                None => {
                    if let Some(floor) = floor
                        && column <= floor
                    {
                        return self.finish_block_scalar(
                            header_span,
                            style,
                            chomp,
                            lines,
                            raw_end,
                            last_terminated,
                            anchor,
                            tag,
                        );
                    }
                    content_indent = Some(column);
                    first_line = pending_blank.unwrap_or(index);
                    break;
                }
            }
        }
        let Some(content_indent) = content_indent else {
            return self.finish_block_scalar(
                header_span,
                style,
                chomp,
                lines,
                raw_end,
                last_terminated,
                anchor,
                tag,
            );
        };

        // Phase 2: consume content lines.
        index = first_line;
        while index < self.line_starts.len() {
            let (text, terminated, end_exclusive) = self.line_at(index);
            if text.trim_start_matches(' ').is_empty() {
                lines.push("");
                raw_end = end_exclusive;
                last_terminated = terminated;
                index += 1;
                continue;
            }
            if self.indent_of(text) < content_indent {
                break;
            }
            lines.push(&text[content_indent.min(text.len())..]);
            raw_end = end_exclusive;
            last_terminated = terminated;
            index += 1;
        }

        self.finish_block_scalar(
            header_span,
            style,
            chomp,
            lines,
            raw_end,
            last_terminated,
            anchor,
            tag,
        )
    }

    fn line_at(&self, index: usize) -> (&'source str, bool, usize) {
        let start = self.line_starts[index];
        let terminated = index + 1 < self.line_starts.len();
        let end_exclusive = if terminated {
            self.line_starts[index + 1]
        } else {
            self.source.len()
        };
        let mut text_end = if terminated {
            end_exclusive - 1
        } else {
            end_exclusive
        };
        if self.source.as_bytes().get(text_end.saturating_sub(1)) == Some(&b'\r')
            && text_end > start
        {
            text_end -= 1;
        }
        (&self.source[start..text_end], terminated, end_exclusive)
    }

    fn indent_of(&self, line: &str) -> usize {
        line.len() - line.trim_start_matches(' ').len()
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_block_scalar(
        &mut self,
        header_span: Span,
        style: ScalarStyle,
        chomp: Chomp,
        lines: Vec<&'source str>,
        raw_end: usize,
        last_terminated: bool,
        anchor: Option<Anchor<'source>>,
        tag: Option<TagHandle<'source>>,
    ) -> Scalar<'source> {
        while self.peek().is_some_and(|token| token.span.start < raw_end) {
            self.advance();
        }
        let decoded = decode_block_scalar(&lines, style, chomp, last_terminated);
        let raw = &self.source[header_span.start..raw_end];
        Scalar::new(
            raw,
            Some(Cow::Owned(decoded)),
            anchor,
            tag,
            Span::new(header_span.start, raw_end),
            style,
            true,
        )
    }
}

fn decode_block_scalar(
    lines: &[&str],
    style: ScalarStyle,
    chomp: Chomp,
    last_terminated: bool,
) -> String {
    let mut out = if style == ScalarStyle::Literal {
        lines.join("\n")
    } else {
        let mut folded = String::new();
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                let previous = lines[index - 1];
                if line.is_empty() {
                    folded.push('\n');
                } else if previous.is_empty() {
                    // The break leaving a blank line is absorbed by folding.
                } else if previous.starts_with([' ', '\t']) || line.starts_with([' ', '\t']) {
                    folded.push('\n');
                } else {
                    folded.push(' ');
                }
            }
            folded.push_str(line);
        }
        folded
    };
    if last_terminated && !lines.is_empty() {
        out.push('\n');
    }
    match chomp {
        Chomp::Strip => {
            while out.ends_with('\n') {
                out.pop();
            }
        }
        Chomp::Clip => {
            while out.ends_with('\n') {
                out.pop();
            }
            if !out.is_empty() {
                out.push('\n');
            }
        }
        Chomp::Keep => {}
    }
    out
}

fn fold_break_runs(inner: &str) -> String {
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\n' || c == '\r' {
            let mut breaks = 1usize;
            loop {
                match chars.peek() {
                    Some('\n') => {
                        breaks += 1;
                        chars.next();
                    }
                    Some('\r') => {
                        breaks += 1;
                        chars.next();
                        if chars.peek() == Some(&'\n') {
                            chars.next();
                        }
                    }
                    _ => break,
                }
            }
            while matches!(chars.peek(), Some(' ') | Some('\t')) {
                chars.next();
            }
            while out.ends_with([' ', '\t']) {
                out.pop();
            }
            if breaks == 1 {
                if !out.is_empty() {
                    out.push(' ');
                }
            } else {
                out.push_str(&"\n".repeat(breaks - 1));
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn decode_single_quoted(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix('\'')?.strip_suffix('\'')?;
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                out.push('\'');
            } else {
                return None;
            }
            continue;
        }
        out.push(c);
    }
    Some(fold_break_runs(&out))
}

fn hex_digits<T: Iterator<Item = char>>(
    chars: &mut std::iter::Peekable<T>,
    count: usize,
) -> Option<u32> {
    let mut value = 0u32;
    for _ in 0..count {
        let digit = chars.next()?;
        let n = digit.to_digit(16)?;
        value = value * 16 + n;
    }
    Some(value)
}

fn decode_double_quoted(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => return None,
            '\\' => {
                let escape = chars.next()?;
                match escape {
                    '0' => out.push('\0'),
                    'a' => out.push('\u{7}'),
                    'b' => out.push('\u{8}'),
                    't' => out.push('\t'),
                    'n' => out.push('\n'),
                    'v' => out.push('\u{b}'),
                    'f' => out.push('\u{c}'),
                    'r' => out.push('\r'),
                    'e' => out.push('\u{1b}'),
                    ' ' => out.push(' '),
                    '"' => out.push('"'),
                    '/' => out.push('/'),
                    '\\' => out.push('\\'),
                    'N' => out.push('\u{85}'),
                    '_' => out.push('\u{a0}'),
                    'L' => out.push('\u{2028}'),
                    'P' => out.push('\u{2029}'),
                    'x' => out.push(char::from_u32(hex_digits(&mut chars, 2)?)?),
                    'u' => out.push(char::from_u32(hex_digits(&mut chars, 4)?)?),
                    'U' => out.push(char::from_u32(hex_digits(&mut chars, 8)?)?),
                    '\n' | '\r' => {
                        if escape == '\r' && chars.peek() == Some(&'\n') {
                            chars.next();
                        }
                        while matches!(
                            chars.peek(),
                            Some(' ') | Some('\t') | Some('\r') | Some('\n')
                        ) {
                            chars.next();
                        }
                    }
                    _ => return None,
                }
            }
            _ => out.push(c),
        }
    }
    Some(fold_break_runs(&out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeKind;

    #[test]
    fn parses_simple_mapping() {
        let source = "name: yaml\nversion: 1.2\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let doc = parsed.document().unwrap();
        let root = doc.root().unwrap();
        assert_eq!(root.kind(), NodeKind::Mapping);
        let mapping = root.as_mapping().unwrap();
        assert_eq!(mapping.len(), 2);
    }

    #[test]
    fn parses_simple_sequence() {
        let source = "- one\n- two\n- three\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let doc = parsed.document().unwrap();
        let root = doc.root().unwrap();
        assert_eq!(root.kind(), NodeKind::Sequence);
        let seq = root.as_sequence().unwrap();
        assert_eq!(seq.len(), 3);
    }

    #[test]
    fn parses_flow_sequence() {
        let source = "[1, 2, 3]\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let doc = parsed.document().unwrap();
        let root = doc.root().unwrap();
        assert_eq!(root.kind(), NodeKind::Sequence);
        let seq = root.as_sequence().unwrap();
        assert_eq!(seq.len(), 3);
    }

    #[test]
    fn parses_flow_mapping() {
        let source = "{name: yaml, version: 1.2}\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let doc = parsed.document().unwrap();
        let root = doc.root().unwrap();
        assert_eq!(root.kind(), NodeKind::Mapping);
        let mapping = root.as_mapping().unwrap();
        assert_eq!(mapping.len(), 2);
    }

    #[test]
    fn parses_nested_mapping() {
        let source = "person:\n  name: Alice\n  age: 30\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let doc = parsed.document().unwrap();
        let root = doc.root().unwrap();
        let mapping = root.as_mapping().unwrap();
        assert_eq!(mapping.len(), 1);
        let entry = &mapping.entries()[0];
        let value = entry.value().unwrap();
        assert_eq!(value.kind(), NodeKind::Mapping);
    }

    #[test]
    fn parses_quoted_scalars() {
        let source = "single: 'hello'\ndouble: \"world\"\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let doc = parsed.document().unwrap();
        let root = doc.root().unwrap();
        let mapping = root.as_mapping().unwrap();
        assert_eq!(mapping.len(), 2);
    }

    #[test]
    fn parses_document_markers() {
        let source = "---\nname: yaml\n...\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let doc = parsed.document().unwrap();
        assert!(doc.start_marker().is_some());
        assert!(doc.end_marker().is_some());
    }

    #[test]
    fn parses_anchor_and_alias() {
        let source = "anchor: &ref value\nalias: *ref\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let doc = parsed.document().unwrap();
        let root = doc.root().unwrap();
        let mapping = root.as_mapping().unwrap();
        assert_eq!(mapping.len(), 2);
        let value = mapping.entries()[0].value().unwrap();
        assert_eq!(value.as_scalar().unwrap().anchor().unwrap().name(), "ref");
        assert_eq!(
            mapping.entries()[1].value().unwrap().kind(),
            NodeKind::Alias
        );
    }

    #[test]
    fn parses_empty_document() {
        let source = "";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let doc = parsed.document().unwrap();
        assert!(doc.root().is_none());
    }

    #[test]
    fn parses_scalar_only() {
        let source = "hello world\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let doc = parsed.document().unwrap();
        let root = doc.root().unwrap();
        assert_eq!(root.kind(), NodeKind::Scalar);
    }

    #[test]
    fn parses_multiple_documents() {
        let source = "---\na: 1\n---\nb: 2\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        assert_eq!(parsed.documents().len(), 2);
    }

    #[test]
    fn parses_sequence_under_key() {
        let source = "list:\n  - 1\n  - 2\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let mapping = parsed
            .document()
            .unwrap()
            .root()
            .unwrap()
            .as_mapping()
            .unwrap();
        let value = mapping.entries()[0].value().unwrap();
        assert_eq!(value.as_sequence().unwrap().len(), 2);
    }

    #[test]
    fn parses_compact_sequence_at_key_column() {
        let source = "a:\n- 1\n- 2\nb: 3\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let mapping = parsed
            .document()
            .unwrap()
            .root()
            .unwrap()
            .as_mapping()
            .unwrap();
        assert_eq!(mapping.len(), 2);
        assert_eq!(
            mapping.entries()[0]
                .value()
                .unwrap()
                .as_sequence()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn decodes_double_quoted_escapes() {
        let source = "s: \"a\\tb\\u0041\\\\\"\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let mapping = parsed
            .document()
            .unwrap()
            .root()
            .unwrap()
            .as_mapping()
            .unwrap();
        let value = mapping.entries()[0].value().unwrap();
        assert_eq!(value.as_scalar().unwrap().decoded(), Some("a\tbA\\"));
    }

    #[test]
    fn folds_multi_line_plain_scalar() {
        let source = "a: one\n  two\nb: x\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let mapping = parsed
            .document()
            .unwrap()
            .root()
            .unwrap()
            .as_mapping()
            .unwrap();
        let value = mapping.entries()[0].value().unwrap();
        assert_eq!(value.as_scalar().unwrap().decoded(), Some("one two"));
        assert_eq!(mapping.len(), 2);
    }

    #[test]
    fn parses_literal_block_scalar() {
        let source = "script: |\n  line1\n  line2\nnext: 1\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let mapping = parsed
            .document()
            .unwrap()
            .root()
            .unwrap()
            .as_mapping()
            .unwrap();
        let value = mapping.entries()[0].value().unwrap();
        assert_eq!(value.as_scalar().unwrap().style(), ScalarStyle::Literal);
        assert_eq!(value.as_scalar().unwrap().decoded(), Some("line1\nline2\n"));
        assert_eq!(mapping.len(), 2);
    }

    #[test]
    fn parses_folded_block_scalar_with_strip() {
        let source = "note: >-\n  a\n  b\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let mapping = parsed
            .document()
            .unwrap()
            .root()
            .unwrap()
            .as_mapping()
            .unwrap();
        let value = mapping.entries()[0].value().unwrap();
        assert_eq!(value.as_scalar().unwrap().decoded(), Some("a b"));
    }

    #[test]
    fn reports_duplicate_keys() {
        let source = "a: 1\na: 2\n";
        let parsed = parse(source);
        assert!(parsed.has_errors());
        assert!(
            parsed
                .diagnostics()
                .iter()
                .any(|d| d.kind.code() == "duplicate-key")
        );
    }

    #[test]
    fn recovers_unterminated_flow_mapping() {
        let source = "{a: 1, b: 2\n";
        let parsed = parse(source);
        assert!(parsed.has_errors());
        assert!(
            parsed
                .diagnostics()
                .iter()
                .any(|d| d.kind.code() == "expected-mapping-end")
        );
        let root = parsed.document().unwrap().root().unwrap();
        assert_eq!(root.as_mapping().unwrap().len(), 2);
    }

    #[test]
    fn deep_nesting_terminates() {
        let mut source = String::new();
        for level in 0..300 {
            source.push_str(&format!("{}-\n", "  ".repeat(level)));
        }
        let parsed = parse(&source);
        assert!(
            parsed
                .diagnostics()
                .iter()
                .any(|d| d.kind.code() == "nesting-limit-exceeded")
        );
    }

    #[test]
    fn parses_explicit_key_entry() {
        let source = "? key\n: value\nother: 1\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let mapping = parsed
            .document()
            .unwrap()
            .root()
            .unwrap()
            .as_mapping()
            .unwrap();
        assert_eq!(mapping.len(), 2);
        assert_eq!(mapping.entries()[0].key().as_str(), Some("key"));
    }

    #[test]
    fn parses_null_key_entry() {
        let source = ": v\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let mapping = parsed
            .document()
            .unwrap()
            .root()
            .unwrap()
            .as_mapping()
            .unwrap();
        assert_eq!(mapping.len(), 1);
        assert_eq!(mapping.entries()[0].key().as_str(), Some(""));
    }

    #[test]
    fn decodes_single_quoted_and_flow_pairs() {
        let source = "s: 'it''s'\np: {a: [1, two], b: c}\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let mapping = parsed
            .document()
            .unwrap()
            .root()
            .unwrap()
            .as_mapping()
            .unwrap();
        assert_eq!(mapping.entries()[0].value().unwrap().as_str(), Some("it's"));
        let pair = mapping.entries()[1].value().unwrap();
        let flow = pair.as_mapping().unwrap();
        assert_eq!(flow.len(), 2);
        assert_eq!(flow.get("a").unwrap().as_sequence().unwrap().len(), 2);
    }

    #[test]
    fn parses_directive_and_keeps_lex_lossless() {
        let source = "%YAML 1.2\n---\nok: true\n";
        let parsed = parse(source);
        assert!(parsed.is_valid());
        let doc = parsed.document().unwrap();
        assert_eq!(doc.directives().len(), 1);
        assert!(
            themoretheless_tokenizer_core::verify_lossless_spans(
                source,
                parsed.lexed().tokens().iter().map(|token| token.span)
            )
            .is_ok()
        );
    }
}
