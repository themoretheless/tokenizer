//! A lossless, context-aware CSS lexer.
//!
//! Token text concatenation always reconstructs the source byte-for-byte,
//! including truncated at-rules, unterminated strings, comments and blocks,
//! stray `}` and empty input. Spans are UTF-8 byte offsets.
//!
//! The lexer carries real CSS context: a brace-nesting stack plus a
//! "selector/prelude vs declaration" mode, because `:` is a pseudo-class in a
//! prelude and a property separator inside a block, and because `#x` is an id
//! selector while `#fff` in a value is a color. Inside a block the mode comes
//! from [`Lexer::item_shape`], which scans ahead to the first `;`, `{` or `}`
//! outside strings, comments and brackets: a `{` means the item is a nested
//! rule, anything else means it is a declaration.
//!
//! Broken atoms become single error-flagged tokens instead of being dropped or
//! split, so recovery only ever has to skip one token. A `;`, `{` or `}`
//! resynchronizes the lexer and releases any unclosed delimiter it was
//! holding; the structural pass in `parser.rs` still reports the delimiters.

use themoretheless_tokenizer_core::{Diagnostic, LosslessViolation, Span, verify_lossless_spans};

/// Exact CSS lexical categories emitted by the lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxKind {
    Whitespace,
    Comment,
    Colon,
    Semicolon,
    Comma,
    LeftBrace,
    RightBrace,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    Operator,
    Dot,
    TypeSelector,
    ClassSelector,
    IdSelector,
    UniversalSelector,
    /// An attribute selector lexes as parts: `[`, this name token, the
    /// matching operator and string, then `]`.
    AttributeSelector,
    PseudoClass,
    PseudoElement,
    NestingSelector,
    Property,
    Variable,
    Important,
    AtRule,
    AtRulePreludeText,
    Function,
    Color,
    Number,
    Percentage,
    Unit,
    String,
    Value,
    Error,
}

impl SyntaxKind {
    /// Whitespace and comments carry no structure for the parse pass.
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(self, Self::Whitespace | Self::Comment)
    }

    /// Kinds that can serve as the value of a declaration.
    #[must_use]
    pub const fn is_value_atom(self) -> bool {
        matches!(
            self,
            Self::Color
                | Self::Number
                | Self::Percentage
                | Self::Unit
                | Self::String
                | Self::Value
                | Self::Variable
                | Self::Function
                | Self::Important
                | Self::UniversalSelector
        )
    }
}

/// Compact per-token state that survives broken input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TokenFlags(u8);

impl TokenFlags {
    /// No flags set.
    pub const EMPTY: Self = Self(0);
    /// The token's text is a broken atom that was kept instead of dropped.
    pub const HAS_ERROR: Self = Self(1);

    /// Whether no flag is set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether every flag in `other` is also set here.
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
    /// The token's source text, or `None` if its span is out of bounds.
    #[must_use]
    pub fn text(self, source: &str) -> Option<&str> {
        source.get(self.span.range())
    }

    /// Whether this token's text is a broken atom.
    #[must_use]
    pub const fn has_error(self) -> bool {
        self.flags.contains(TokenFlags::HAS_ERROR)
    }
}

/// Lossless lexer output: every source byte belongs to exactly one token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lexed<'source> {
    source: &'source str,
    tokens: Vec<LexToken>,
    diagnostics: Vec<Diagnostic>,
}

impl<'source> Lexed<'source> {
    /// The exact source this stream was lexed from.
    #[must_use]
    pub const fn source(&self) -> &'source str {
        self.source
    }

    /// Every token, trivia included, in ascending span order.
    #[must_use]
    pub fn tokens(&self) -> &[LexToken] {
        &self.tokens
    }

    /// Tokens with whitespace and comments removed.
    pub fn significant_tokens(&self) -> impl Iterator<Item = LexToken> + '_ {
        self.tokens
            .iter()
            .copied()
            .filter(|token| !token.kind.is_trivia())
    }

    /// Diagnostics found while lexing: unterminated atoms and bad literals.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Convenience for `token.text(self.source())`.
    #[must_use]
    pub fn text(&self, token: LexToken) -> Option<&'source str> {
        token.text(self.source)
    }

    /// Whether any token is error-flagged or any diagnostic was raised.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty() || self.tokens.iter().any(|token| token.has_error())
    }

    /// Whether the token spans tile the source without gaps or overlaps.
    pub fn verify_lossless(&self) -> Result<(), LosslessViolation> {
        verify_lossless_spans(self.source, self.tokens.iter().map(|token| token.span))
    }

    /// Whether concatenating token text rebuilds `source` exactly.
    #[must_use]
    pub fn is_lossless(&self, source: &str) -> bool {
        self.verify_lossless().is_ok() && self.joined() == source
    }

    /// Concatenated token text, which must equal the source.
    #[must_use]
    pub fn joined(&self) -> String {
        let mut out = String::with_capacity(self.source.len());
        for token in &self.tokens {
            if let Some(text) = token.text(self.source) {
                out.push_str(text);
            }
        }
        out
    }
}

/// Lexes a stylesheet losslessly.
#[must_use]
pub fn lex(source: &str) -> Lexed<'_> {
    Lexer {
        source,
        bytes: source.as_bytes(),
        cursor: 0,
        tokens: Vec::new(),
        diagnostics: Vec::new(),
        mode: Mode::Selector,
        depth: 0,
        delims: Vec::new(),
        at_item_start: true,
        property_seen: false,
        statement_at_rule: false,
        at_keyword: Span::new(0, 0),
    }
    .run()
}

/// Which CSS production the cursor sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// A selector or rule prelude: `#x` is an id, `:` starts a pseudo-class.
    Selector,
    /// A declaration inside a block, before its colon.
    Declaration,
    /// After a declaration colon.
    Value,
    /// After an at-keyword, up to the block or `;` that ends the statement.
    AtPrelude,
}

/// What the item at a block boundary turns out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemShape {
    NestedRule,
    Declaration,
}

/// One open `(` or `[`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delim {
    Paren,
    Bracket,
}

/// At-rules that are statements: they end in `;`, never in `{`.
const STATEMENT_AT_RULES: &[&str] = &["import", "charset", "namespace", "use", "forward"];

struct Lexer<'source> {
    source: &'source str,
    bytes: &'source [u8],
    cursor: usize,
    tokens: Vec<LexToken>,
    diagnostics: Vec<Diagnostic>,
    mode: Mode,
    depth: usize,
    delims: Vec<Delim>,
    at_item_start: bool,
    property_seen: bool,
    statement_at_rule: bool,
    at_keyword: Span,
}

impl<'source> Lexer<'source> {
    fn run(mut self) -> Lexed<'source> {
        while self.cursor < self.bytes.len() {
            let start = self.cursor;
            self.lex_token();
            debug_assert!(self.cursor > start, "the lexer must always make progress");
        }
        self.finish();
        self.diagnostics
            .sort_by_key(|diagnostic| diagnostic.span.start);
        Lexed {
            source: self.source,
            tokens: self.tokens,
            diagnostics: self.diagnostics,
        }
    }

    fn lex_token(&mut self) {
        let byte = self.at(self.cursor).unwrap_or(b' ');
        if is_whitespace(byte) {
            self.lex_whitespace();
            return;
        }
        if byte == b'/' && self.starts_with(self.cursor, b"/*") {
            self.lex_comment();
            return;
        }
        if self.at_item_start && !matches!(byte, b'}' | b';') {
            self.begin_item();
        }
        match byte {
            b'{' => self.lex_open_brace(),
            b'}' => self.lex_close_brace(),
            b';' => self.lex_semicolon(),
            b',' => self.single(SyntaxKind::Comma),
            b':' => self.lex_colon(),
            b'.' => self.lex_dot(),
            b'#' => self.lex_hash(),
            b'@' => self.lex_at_keyword(),
            b'&' => {
                let kind = if self.mode == Mode::Selector {
                    SyntaxKind::NestingSelector
                } else {
                    SyntaxKind::Operator
                };
                self.single(kind);
            }
            b'*' if self.mode == Mode::Selector && self.at(self.cursor + 1) != Some(b'=') => {
                self.single(SyntaxKind::UniversalSelector);
            }
            b'!' => self.lex_bang(),
            b'"' | b'\'' => self.lex_string(byte),
            b'0'..=b'9' => self.lex_number(),
            b'(' => {
                self.delims.push(Delim::Paren);
                self.single(SyntaxKind::LeftParen);
            }
            b')' => {
                self.delims.pop();
                self.single(SyntaxKind::RightParen);
            }
            b'[' => {
                self.delims.push(Delim::Bracket);
                self.single(SyntaxKind::LeftBracket);
            }
            b']' => {
                self.delims.pop();
                self.single(SyntaxKind::RightBracket);
            }
            b'+' | b'-' if self.number_starts_at(self.cursor) => self.lex_number(),
            _ if self.ident_start_at(self.cursor) => self.lex_ident(),
            _ if is_operator_start(byte) => self.lex_operator_run(),
            _ => self.lex_unexpected(),
        }
    }

    /// Chooses the mode for the item that starts at the cursor.
    fn begin_item(&mut self) {
        self.at_item_start = false;
        self.property_seen = false;
        if self.depth == 0 {
            self.mode = Mode::Selector;
            return;
        }
        self.mode = match self.item_shape() {
            ItemShape::NestedRule => Mode::Selector,
            ItemShape::Declaration => Mode::Declaration,
        };
    }

    /// Scans the current block item to its first top-level delimiter.
    ///
    /// A declaration can never contain a `{`, so finding one first means the
    /// item is a nested rule. That is what makes `&:hover { … }` lex as a
    /// selector while `color: red;` next to it lexes as a declaration.
    fn item_shape(&self) -> ItemShape {
        let mut index = self.cursor;
        let mut depth = 0usize;
        while index < self.bytes.len() {
            let byte = self.bytes[index];
            match byte {
                b'/' if self.starts_with(index, b"/*") => index = self.comment_end(index + 2),
                b'"' | b'\'' => index = self.string_end(index + 1, byte),
                b'(' | b'[' => {
                    depth += 1;
                    index += 1;
                }
                b')' | b']' => {
                    depth = depth.saturating_sub(1);
                    index += 1;
                }
                b'{' if depth == 0 => return ItemShape::NestedRule,
                b';' | b'}' if depth == 0 => return ItemShape::Declaration,
                _ => index += 1,
            }
        }
        ItemShape::Declaration
    }

    fn lex_whitespace(&mut self) {
        let start = self.cursor;
        while self.cursor < self.bytes.len() && is_whitespace(self.at(self.cursor).unwrap_or(b' '))
        {
            self.cursor += 1;
        }
        self.push(SyntaxKind::Whitespace, start, self.cursor, false);
    }

    fn lex_comment(&mut self) {
        let start = self.cursor;
        let end = self.comment_end(start + 2);
        let closed = self.starts_with(end.saturating_sub(2), b"*/");
        self.cursor = end;
        self.push(SyntaxKind::Comment, start, end, !closed);
        if !closed {
            self.diagnostic(
                start,
                end,
                "unclosed-comment",
                "comment is missing its closing `*/`",
            );
        }
    }

    /// First offset after the `*/` that closes a comment body starting at `from`.
    fn comment_end(&self, from: usize) -> usize {
        let mut index = from.min(self.bytes.len());
        while index < self.bytes.len() {
            if self.starts_with(index, b"*/") {
                return index + 2;
            }
            index += 1;
        }
        self.bytes.len()
    }

    fn lex_open_brace(&mut self) {
        let start = self.cursor;
        self.report_unterminated_statement_at_rule();
        self.statement_at_rule = false;
        self.cursor = start + 1;
        self.push(SyntaxKind::LeftBrace, start, self.cursor, false);
        self.depth += 1;
        self.delims.clear();
        self.at_item_start = true;
    }

    fn lex_close_brace(&mut self) {
        let start = self.cursor;
        self.report_unterminated_statement_at_rule();
        self.statement_at_rule = false;
        self.cursor = start + 1;
        self.push(SyntaxKind::RightBrace, start, self.cursor, false);
        self.depth = self.depth.saturating_sub(1);
        self.delims.clear();
        self.at_item_start = true;
    }

    fn lex_semicolon(&mut self) {
        let start = self.cursor;
        self.statement_at_rule = false;
        self.cursor = start + 1;
        self.push(SyntaxKind::Semicolon, start, self.cursor, false);
        self.delims.clear();
        self.at_item_start = true;
    }

    fn lex_colon(&mut self) {
        let start = self.cursor;
        if self.mode == Mode::Selector && self.ident_start_at(start + 1) {
            let end = self.scan_ident_end(start + 1);
            self.cursor = end;
            self.push(SyntaxKind::PseudoClass, start, end, false);
            return;
        }
        if self.mode == Mode::Selector
            && self.starts_with(start, b"::")
            && self.ident_start_at(start + 2)
        {
            let end = self.scan_ident_end(start + 2);
            self.cursor = end;
            self.push(SyntaxKind::PseudoElement, start, end, false);
            return;
        }
        if self.mode == Mode::Declaration {
            self.mode = Mode::Value;
        }
        self.cursor = start + 1;
        self.push(SyntaxKind::Colon, start, self.cursor, false);
    }

    fn lex_dot(&mut self) {
        let start = self.cursor;
        if self.mode == Mode::Selector && self.ident_start_at(start + 1) {
            let end = self.scan_ident_end(start + 1);
            self.cursor = end;
            self.push(SyntaxKind::ClassSelector, start, end, false);
            return;
        }
        if self.at(start + 1).is_some_and(|byte| byte.is_ascii_digit()) {
            self.lex_number();
            return;
        }
        self.cursor = start + 1;
        self.push(SyntaxKind::Dot, start, self.cursor, false);
    }

    /// `#` is an id selector in a prelude and a color literal in a value.
    fn lex_hash(&mut self) {
        let start = self.cursor;
        if self.mode == Mode::Selector {
            if self.ident_start_at(start + 1) {
                let end = self.scan_ident_end(start + 1);
                self.cursor = end;
                self.push(SyntaxKind::IdSelector, start, end, false);
                return;
            }
            self.cursor = start + 1;
            self.push(SyntaxKind::Error, start, self.cursor, true);
            return;
        }
        let end = self.scan_ident_end(start + 1);
        let digits = &self.bytes[start + 1..end];
        let valid = digits.len() <= 8
            && matches!(digits.len(), 3 | 4 | 6 | 8)
            && digits.iter().all(u8::is_ascii_hexdigit);
        self.cursor = end;
        self.push(SyntaxKind::Color, start, end, !valid);
        if !valid {
            self.diagnostic(
                start,
                end,
                "invalid-hex-color",
                "hex color needs 3, 4, 6 or 8 hexadecimal digits",
            );
        }
    }

    fn lex_at_keyword(&mut self) {
        let start = self.cursor;
        if !self.ident_start_at(start + 1) {
            self.cursor = start + 1;
            self.push(SyntaxKind::Error, start, self.cursor, true);
            return;
        }
        let end = self.scan_ident_end(start + 1);
        self.at_keyword = Span::new(start, end);
        self.statement_at_rule = STATEMENT_AT_RULES
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&self.source[start + 1..end]));
        self.mode = Mode::AtPrelude;
        self.cursor = end;
        self.push(SyntaxKind::AtRule, start, end, false);
    }

    /// `!` plus its optional whitespace plus `important` is one token.
    fn lex_bang(&mut self) {
        let start = self.cursor;
        if self.mode == Mode::Value {
            let mut index = start + 1;
            while self.at(index).is_some_and(is_whitespace) {
                index += 1;
            }
            let name_end = self.scan_ident_end(index);
            if name_end > index
                && self
                    .source
                    .get(index..name_end)
                    .is_some_and(|text| text.eq_ignore_ascii_case("important"))
            {
                self.cursor = name_end;
                self.push(SyntaxKind::Important, start, name_end, false);
                return;
            }
        }
        self.cursor = start + 1;
        self.push(SyntaxKind::Operator, start, self.cursor, false);
    }

    fn lex_string(&mut self, quote: u8) {
        let start = self.cursor;
        let end = self.string_end(start + 1, quote);
        let terminated = end >= start + 2 && self.at(end - 1) == Some(quote);
        self.cursor = end;
        self.push(SyntaxKind::String, start, end, !terminated);
        if !terminated {
            self.diagnostic(
                start,
                end,
                "unclosed-string",
                "string is not terminated before the end of the line",
            );
        }
    }

    /// First offset after a string body that starts at `from`.
    fn string_end(&self, from: usize, quote: u8) -> usize {
        let mut index = from.min(self.bytes.len());
        while index < self.bytes.len() {
            match self.bytes[index] {
                b'\\' => index += 1 + self.char_len(index + 1),
                b'\n' | b'\r' => return index,
                byte if byte == quote => return index + 1,
                _ => index += 1,
            }
        }
        self.bytes.len()
    }

    fn lex_number(&mut self) {
        let start = self.cursor;
        let Some(end) = self.scan_number_end(start) else {
            self.lex_operator_run();
            return;
        };
        if self.at(end) == Some(b'%') {
            self.cursor = end + 1;
            self.push(SyntaxKind::Percentage, start, self.cursor, false);
            return;
        }
        self.cursor = end;
        self.push(SyntaxKind::Number, start, end, false);
        if self.unit_start_at(end) {
            let unit_end = self.scan_ident_end(end);
            self.cursor = unit_end;
            self.push(SyntaxKind::Unit, end, unit_end, false);
        }
    }

    /// Consumes a number, returning its end only when it has a digit.
    fn scan_number_end(&self, from: usize) -> Option<usize> {
        let mut index = from;
        if matches!(self.at(index), Some(b'+') | Some(b'-')) {
            index += 1;
        }
        let mut digits = 0;
        while self.at(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
            digits += 1;
        }
        if digits == 0 {
            if self.at(index) == Some(b'.')
                && self.at(index + 1).is_some_and(|byte| byte.is_ascii_digit())
            {
                index += 1;
                while self.at(index).is_some_and(|byte| byte.is_ascii_digit()) {
                    index += 1;
                }
            } else {
                return None;
            }
        } else if self.at(index) == Some(b'.')
            && !self
                .at(index + 1)
                .is_some_and(|byte| byte.is_ascii_alphabetic())
        {
            index += 1;
            while self.at(index).is_some_and(|byte| byte.is_ascii_digit()) {
                index += 1;
            }
        }
        if matches!(self.at(index), Some(b'e') | Some(b'E')) {
            let mut exponent = index + 1;
            if matches!(self.at(exponent), Some(b'+') | Some(b'-')) {
                exponent += 1;
            }
            let mut exponent_digits = 0;
            while self.at(exponent).is_some_and(|byte| byte.is_ascii_digit()) {
                exponent += 1;
                exponent_digits += 1;
            }
            if exponent_digits > 0 {
                index = exponent;
            }
        }
        Some(index)
    }

    fn number_starts_at(&self, from: usize) -> bool {
        let mut index = from;
        if matches!(self.at(index), Some(b'+') | Some(b'-')) {
            index += 1;
        }
        self.at(index).is_some_and(|byte| byte.is_ascii_digit())
            || (self.at(index) == Some(b'.')
                && self.at(index + 1).is_some_and(|byte| byte.is_ascii_digit()))
    }

    /// A unit is an ident glued straight onto a number, like `ms` in `300ms`.
    fn unit_start_at(&self, from: usize) -> bool {
        self.at(from)
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80)
    }

    fn lex_ident(&mut self) {
        let start = self.cursor;
        let mut end = self.scan_ident_end(start);
        let kind = self.classify_ident(end);
        if kind == SyntaxKind::TypeSelector && self.namespace_continues(end) {
            end = self.scan_ident_end(end + 1);
        }
        if kind == SyntaxKind::Property {
            self.property_seen = true;
        }
        self.cursor = end;
        self.push(kind, start, end, false);
    }

    /// Turns the ident ending at `end` into its context-dependent kind.
    fn classify_ident(&mut self, end: usize) -> SyntaxKind {
        if self.starts_with(self.cursor, b"--") {
            return SyntaxKind::Variable;
        }
        if self.at(end) == Some(b'(') {
            return SyntaxKind::Function;
        }
        match self.mode {
            Mode::Selector => {
                if self.delims.last() == Some(&Delim::Bracket) {
                    SyntaxKind::AttributeSelector
                } else if self.delims.is_empty() {
                    SyntaxKind::TypeSelector
                } else {
                    SyntaxKind::Value
                }
            }
            Mode::Declaration => {
                if self.property_seen {
                    SyntaxKind::Value
                } else {
                    SyntaxKind::Property
                }
            }
            Mode::Value => {
                // A second `property:` on one line means the previous
                // declaration forgot its `;`. Only outside delimiters, so an
                // unquoted `url(http://…)` stays intact.
                if self.delims.is_empty() && self.next_significant_is_colon(end) {
                    SyntaxKind::Property
                } else {
                    SyntaxKind::Value
                }
            }
            Mode::AtPrelude => SyntaxKind::AtRulePreludeText,
        }
    }

    fn namespace_continues(&self, end: usize) -> bool {
        self.at(end) == Some(b'|')
            && self.at(end + 1) != Some(b'|')
            && (self.ident_start_at(end + 1) || self.at(end + 1) == Some(b'*'))
    }

    fn next_significant_is_colon(&self, from: usize) -> bool {
        let mut index = from;
        while index < self.bytes.len() {
            let byte = self.bytes[index];
            if is_whitespace(byte) {
                index += 1;
            } else if self.starts_with(index, b"/*") {
                index = self.comment_end(index + 2);
            } else {
                return byte == b':' && self.at(index + 1) != Some(b':');
            }
        }
        false
    }

    /// Ident start, where a leading `-` must be followed by a name.
    fn ident_start_at(&self, from: usize) -> bool {
        match self.at(from) {
            None => false,
            Some(b'\\') => !matches!(self.at(from + 1), None | Some(b'\n') | Some(b'\r')),
            Some(b'-') => match self.at(from + 1) {
                Some(b'-' | b'\\') => true,
                Some(byte) => byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80,
                None => false,
            },
            Some(byte) => byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80,
        }
    }

    fn scan_ident_end(&self, from: usize) -> usize {
        let mut index = from.min(self.bytes.len());
        while index < self.bytes.len() {
            let byte = self.bytes[index];
            if byte == b'\\' {
                index += 1;
                if !matches!(self.at(index), None | Some(b'\n') | Some(b'\r')) {
                    index += self.char_len(index);
                }
            } else if byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' || byte >= 0x80 {
                index += 1;
            } else {
                break;
            }
        }
        index
    }

    fn lex_operator_run(&mut self) {
        let start = self.cursor;
        let mut index = start;
        while index < self.bytes.len() && is_operator_char(self.bytes[index]) {
            index += 1;
        }
        if index == start {
            index += 1;
        }
        self.cursor = index;
        self.push(SyntaxKind::Operator, start, index, false);
    }

    fn lex_unexpected(&mut self) {
        let start = self.cursor;
        let end = start + self.char_len(start);
        self.cursor = end;
        self.push(SyntaxKind::Error, start, end, true);
    }

    /// A statement at-rule (`@import`, `@charset`) must end in `;`.
    fn report_unterminated_statement_at_rule(&mut self) {
        if !self.statement_at_rule || self.mode != Mode::AtPrelude {
            return;
        }
        if self.at_keyword.end == self.cursor {
            return;
        }
        self.diagnostic_span(
            self.at_keyword,
            "missing-semicolon",
            "at-rule statement is missing its `;`",
        );
    }

    /// Catches an unclosed statement at-rule that ran into EOF.
    fn finish(&mut self) {
        self.report_unterminated_statement_at_rule();
    }

    fn single(&mut self, kind: SyntaxKind) {
        let start = self.cursor;
        self.cursor = start + 1;
        self.push(kind, start, self.cursor, false);
    }

    fn push(&mut self, kind: SyntaxKind, start: usize, end: usize, error: bool) {
        debug_assert!(start < end, "CSS tokens are never empty");
        if start >= end {
            return;
        }
        self.tokens.push(LexToken {
            kind,
            span: Span::new(start, end),
            flags: if error {
                TokenFlags::HAS_ERROR
            } else {
                TokenFlags::EMPTY
            },
        });
    }

    fn diagnostic(&mut self, start: usize, end: usize, code: &'static str, message: &'static str) {
        self.diagnostic_span(Span::new(start, end), code, message);
    }

    fn diagnostic_span(&mut self, span: Span, code: &'static str, message: &'static str) {
        self.diagnostics.push(Diagnostic::new(span, code, message));
    }

    fn at(&self, index: usize) -> Option<u8> {
        self.bytes.get(index).copied()
    }

    fn starts_with(&self, at: usize, pattern: &[u8]) -> bool {
        self.bytes.len() >= at + pattern.len() && &self.bytes[at..at + pattern.len()] == pattern
    }

    /// UTF-8 length of the character at `index`, keeping every slice on a
    /// char boundary.
    fn char_len(&self, index: usize) -> usize {
        self.source
            .get(index..)
            .and_then(|tail| tail.chars().next())
            .map_or(1, char::len_utf8)
    }
}

fn is_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}

fn is_operator_start(byte: u8) -> bool {
    matches!(
        byte,
        b'>' | b'<'
            | b'+'
            | b'~'
            | b'|'
            | b'='
            | b'^'
            | b'$'
            | b'/'
            | b'%'
            | b'-'
            | b'*'
            | b'!'
            | b'&'
    )
}

fn is_operator_char(byte: u8) -> bool {
    matches!(
        byte,
        b'>' | b'<'
            | b'+'
            | b'~'
            | b'|'
            | b'='
            | b'^'
            | b'$'
            | b'/'
            | b'%'
            | b'-'
            | b'*'
            | b'!'
            | b'&'
    )
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

    fn pairs(source: &str) -> Vec<(SyntaxKind, &str)> {
        let lexed = lex(source);
        lexed
            .significant_tokens()
            .map(|token| (token.kind, token.text(lexed.source()).unwrap()))
            .collect()
    }

    #[test]
    fn property_and_selector_are_distinguishable() {
        let kinds = kinds("body { color: red; }");
        assert!(kinds.contains(&SyntaxKind::TypeSelector));
        assert!(kinds.contains(&SyntaxKind::Property));
        assert!(kinds.contains(&SyntaxKind::Value));
    }

    #[test]
    fn hash_is_an_id_in_a_selector_and_a_color_in_a_value() {
        assert_eq!(
            pairs("#title { color: #fff; }"),
            vec![
                (SyntaxKind::IdSelector, "#title"),
                (SyntaxKind::LeftBrace, "{"),
                (SyntaxKind::Property, "color"),
                (SyntaxKind::Colon, ":"),
                (SyntaxKind::Color, "#fff"),
                (SyntaxKind::Semicolon, ";"),
                (SyntaxKind::RightBrace, "}"),
            ]
        );
    }

    #[test]
    fn pseudo_colons_do_not_start_a_value() {
        assert_eq!(
            pairs("a:hover::before { color: red; }"),
            vec![
                (SyntaxKind::TypeSelector, "a"),
                (SyntaxKind::PseudoClass, ":hover"),
                (SyntaxKind::PseudoElement, "::before"),
                (SyntaxKind::LeftBrace, "{"),
                (SyntaxKind::Property, "color"),
                (SyntaxKind::Colon, ":"),
                (SyntaxKind::Value, "red"),
                (SyntaxKind::Semicolon, ";"),
                (SyntaxKind::RightBrace, "}"),
            ]
        );
    }

    #[test]
    fn dimensions_split_number_and_unit() {
        assert_eq!(
            pairs("a { transition: 300ms 1.5s; }"),
            vec![
                (SyntaxKind::TypeSelector, "a"),
                (SyntaxKind::LeftBrace, "{"),
                (SyntaxKind::Property, "transition"),
                (SyntaxKind::Colon, ":"),
                (SyntaxKind::Number, "300"),
                (SyntaxKind::Unit, "ms"),
                (SyntaxKind::Number, "1.5"),
                (SyntaxKind::Unit, "s"),
                (SyntaxKind::Semicolon, ";"),
                (SyntaxKind::RightBrace, "}"),
            ]
        );
    }

    #[test]
    fn nth_child_an_plus_b() {
        let lexed = lex("li:nth-child(2n+1) { color: red; }");
        assert_eq!(lexed.diagnostics(), []);
        let kinds: Vec<SyntaxKind> = lexed
            .significant_tokens()
            .map(|token| token.kind)
            .filter(|kind| {
                matches!(
                    kind,
                    SyntaxKind::PseudoClass | SyntaxKind::Number | SyntaxKind::Unit
                )
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                SyntaxKind::PseudoClass,
                SyntaxKind::Number,
                SyntaxKind::Unit,
                SyntaxKind::Number
            ]
        );
    }

    #[test]
    fn important_span_includes_the_marker() {
        let lexed = lex("a { color: red ! important; }");
        let token = lexed
            .significant_tokens()
            .find(|token| token.kind == SyntaxKind::Important)
            .unwrap();
        assert_eq!(token.text(lexed.source()), Some("! important"));
    }

    #[test]
    fn nested_rules_take_the_selector_branch() {
        let text = pairs("a { & b { color: red; } }");
        assert!(text.contains(&(SyntaxKind::NestingSelector, "&")));
        assert!(text.contains(&(SyntaxKind::TypeSelector, "b")));
        assert!(text.contains(&(SyntaxKind::Property, "color")));
    }

    #[test]
    fn custom_properties_are_variables_in_definition_and_use() {
        let text = pairs(":root { --brand: #0a6; } a { color: var(--brand); }");
        let variables = text
            .iter()
            .filter(|(kind, _)| *kind == SyntaxKind::Variable)
            .map(|(_, text)| *text)
            .collect::<Vec<_>>();
        assert_eq!(variables, vec!["--brand", "--brand"]);
    }

    #[test]
    fn quoted_semicolons_do_not_end_a_declaration() {
        let lexed = lex("a { content: \"; }\"; color: red; }");
        assert_eq!(lexed.diagnostics(), []);
        assert_eq!(
            lexed
                .significant_tokens()
                .filter(|token| token.kind == SyntaxKind::Property)
                .count(),
            2
        );
    }

    #[test]
    fn unquoted_url_keeps_its_colon() {
        let lexed = lex("a { background: url(http://x/y.png); color: red; }");
        assert_eq!(lexed.diagnostics(), []);
    }

    #[test]
    fn at_rule_prelude_and_media_feature() {
        assert_eq!(
            pairs("@media (max-width: 600px) { a { b: c; } }"),
            vec![
                (SyntaxKind::AtRule, "@media"),
                (SyntaxKind::LeftParen, "("),
                (SyntaxKind::AtRulePreludeText, "max-width"),
                (SyntaxKind::Colon, ":"),
                (SyntaxKind::Number, "600"),
                (SyntaxKind::Unit, "px"),
                (SyntaxKind::RightParen, ")"),
                (SyntaxKind::LeftBrace, "{"),
                (SyntaxKind::TypeSelector, "a"),
                (SyntaxKind::LeftBrace, "{"),
                (SyntaxKind::Property, "b"),
                (SyntaxKind::Colon, ":"),
                (SyntaxKind::Value, "c"),
                (SyntaxKind::Semicolon, ";"),
                (SyntaxKind::RightBrace, "}"),
                (SyntaxKind::RightBrace, "}"),
            ]
        );
    }

    #[test]
    fn attribute_selector_lexes_as_parts() {
        assert_eq!(
            pairs("a[href^=\"x\"] { color: red; }"),
            vec![
                (SyntaxKind::TypeSelector, "a"),
                (SyntaxKind::LeftBracket, "["),
                (SyntaxKind::AttributeSelector, "href"),
                (SyntaxKind::Operator, "^="),
                (SyntaxKind::String, "\"x\""),
                (SyntaxKind::RightBracket, "]"),
                (SyntaxKind::LeftBrace, "{"),
                (SyntaxKind::Property, "color"),
                (SyntaxKind::Colon, ":"),
                (SyntaxKind::Value, "red"),
                (SyntaxKind::Semicolon, ";"),
                (SyntaxKind::RightBrace, "}"),
            ]
        );
    }

    #[test]
    fn namespaced_type_selector_is_one_token() {
        let tokens = pairs("svg|rect { color: red; }");
        assert_eq!(tokens[0], (SyntaxKind::TypeSelector, "svg|rect"));
        assert!(!tokens.iter().any(|(kind, _)| *kind == SyntaxKind::Operator));
    }

    #[test]
    fn non_ascii_stays_on_char_boundaries() {
        let lexed = lex(".перевод { content: \"→\"; color: red; }");
        assert!(lexed.verify_lossless().is_ok());
        assert!(
            lexed
                .significant_tokens()
                .any(|token| token.kind == SyntaxKind::ClassSelector)
        );
    }

    #[test]
    fn broken_atoms_are_flagged_not_dropped() {
        for source in [
            "a { content: \"x",
            "/* cut",
            "a { color: #gg; }",
            "a { color: #12345; }",
            "a { color: #ff; }",
        ] {
            let lexed = lex(source);
            assert!(lexed.has_errors(), "{source}");
            assert!(lexed.verify_lossless().is_ok(), "{source}");
        }
    }

    #[test]
    fn weird_input_never_panics() {
        for source in [
            "",
            "}",
            "}}}",
            "a {",
            "@",
            "@media",
            "@media (max-width:",
            "\t\r\n",
            "/*!*/",
            "/*!",
            "a{}b{}",
            "$foo",
            "\u{feff}",
            "\\",
            "a\\",
            "content:",
            ":",
            "::",
            ";",
            "[",
            "]",
            "(",
            ")",
            "#",
            "1.5.5",
            "--",
            "a { & {}",
            "e10",
            "1e",
            "1e+",
            "@media{a{b:c}}",
        ] {
            let lexed = lex(source);
            assert!(lexed.verify_lossless().is_ok(), "{source:?}");
        }
    }

    #[test]
    fn crlf_and_tabs_survive() {
        let source = "a {\r\n\tcolor:\r\n\t\tred;\r\n}\r\n";
        let lexed = lex(source);
        assert!(lexed.is_lossless(source));
    }
}
