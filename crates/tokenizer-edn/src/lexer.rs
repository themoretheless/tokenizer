//! A lossless EDN lexer.
//!
//! Concatenating token text reconstructs the source byte_for_byte, including
//! for malformed input: an unterminated string, a stray `\`, a `#` with no
//! dispatch suffix all keep their own span — flagged [`LexToken::has_error`]
//! with a stable diagnostic — and are never dropped or replaced by a
//! synthesized token. Nothing here borrows a programming_language vocabulary:
//! EDN has `symbol`s and `keyword`s, `instant_tag`s and `uuid_tag`s, a
//! `set_open` that is two bytes, `radix_integer`s and `ratio`s, because this
//! is the data notation's own grammar, not a Lisp language's.
//!
//! The grammar is the EDN specification (edn_format), with the de_facto
//! extensions the format's readers accept in data: radix integers
//! (`2r1010`), ratios (`3/4`), the special numeric values (`##Inf`, `##-Inf`,
//! `##NaN`), the byte_literal tag (`#b`) and the namespaced_map prefix
//! (`#:ns`). Clojure-only reader macros (`'`, `` ` ``, `@`, `^`, `#(`, `#'`,
//! `#=`, `::kw`) are *not* data: the lexer keeps their bytes as one `error`
//! token and blames them with `non_edn_construct`.
//!
//! Per the spec, commas are whitespace, `;` starts a line comment, symbols
//! may contain `: # @` as interior characters (never leading), and `/` is a
//! legal symbol on its own and a namespace separator otherwise.

use themoretheless_tokenizer_core::{
    Diagnostic, DiagnosticKind as _, LosslessViolation, Span, verify_lossless_spans,
};

use crate::parser::DiagnosticKind;

/// Exact lexical categories emitted by the EDN lexer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxKind {
    /// The UTF-8 encoding of U+FEFF at document start.
    Bom,
    /// A run of spaces, tabs, newlines, form feeds, carriage returns and
    /// commas — commas are whitespace in EDN.
    Whitespace,
    /// A `;` to end_of_line comment, the `;` included.
    Comment,
    /// A double_quoted string region, the surrounding quotes included.
    String,
    /// A `\t`, `\\`, `\"`, `\uXXXX`… sequence inside a string.
    StringEscape,
    /// A character literal: `\c`, `\newline`, `\u00e9`, `\(`.
    Character,
    /// A plain symbol (`user`, `+`, `..`, `foo@bar`, the lone `/`).
    Symbol,
    /// A symbol with exactly one interior slash: `my.namespace/foo`.
    NamespacedSymbol,
    /// A keyword: `:name`.
    Keyword,
    /// A namespaced keyword: `:ns/name`.
    NamespacedKeyword,
    /// `true` or `false`.
    BooleanLiteral,
    /// `nil`.
    NilLiteral,
    /// An integer: `42`, `-7`, `+5`.
    Integer,
    /// An arbitrary-precision integer: `1000N`.
    BigInteger,
    /// A radix integer: `2r1010`, `16rFF`, optionally `N`-suffixed.
    RadixInteger,
    /// A floating-point number: `1.5`, `6.022e23`.
    Float,
    /// An exact decimal: `5M`, `1.5M`, `1e5M`.
    Decimal,
    /// A ratio of two integers: `3/4`.
    Ratio,
    /// One of `##Inf`, `##-Inf`, `##NaN`.
    SpecialNumber,
    /// The `(` byte.
    ListOpen,
    /// The `)` byte.
    ListClose,
    /// The `[` byte.
    VectorOpen,
    /// The `]` byte.
    VectorClose,
    /// The `{` byte.
    MapOpen,
    /// The `}` byte — it closes both maps and sets.
    MapClose,
    /// The two-byte `#{` set delimiter.
    SetOpen,
    /// The two-byte `#_` discard sequence.
    Discard,
    /// A user tag: `#` plus a symbol, e.g. `#myco/Person`.
    Tag,
    /// The built-in `#inst` tag.
    InstantTag,
    /// The built-in `#uuid` tag.
    UuidTag,
    /// The byte-literal `#b` tag.
    ByteTag,
    /// A namespaced-map prefix: `#:` plus a symbol, e.g. `#:app`.
    NamespacedMapPrefix,
    /// A span the lexer flagged but kept: a reader macro, a broken number,
    /// an unterminated string region, a stray byte.
    Error,
}

impl SyntaxKind {
    /// Trivia separates elements without carrying EDN content.
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(self, Self::Bom | Self::Whitespace | Self::Comment)
    }

    /// Kinds that open a delimited form.
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(
            self,
            Self::ListOpen | Self::VectorOpen | Self::MapOpen | Self::SetOpen
        )
    }

    /// Kinds that close a delimited form.
    #[must_use]
    pub const fn is_close(self) -> bool {
        matches!(self, Self::ListClose | Self::VectorClose | Self::MapClose)
    }

    /// The close delimiter an open delimiter expects.
    #[must_use]
    pub const fn expected_close(open: Self) -> Self {
        match open {
            Self::ListOpen => Self::ListClose,
            Self::VectorOpen => Self::VectorClose,
            // Sets and maps share the `}` byte.
            Self::MapOpen | Self::SetOpen => Self::MapClose,
            _ => Self::Error,
        }
    }

    /// Kinds carrying `#tag`, including the built-ins and `#b`.
    #[must_use]
    pub const fn is_tag(self) -> bool {
        matches!(
            self,
            Self::Tag | Self::InstantTag | Self::UuidTag | Self::ByteTag
        )
    }
}

/// Compact per-token state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TokenFlags(u8);

impl TokenFlags {
    pub const EMPTY: Self = Self(0);
    /// The span is part of, or abandoned by, a malformed construct.
    pub const HAS_ERROR: Self = Self(1);
    /// The token continues the previous token's construct — a string
    /// region's tail or carved-out escape — and is not its own element.
    pub const CONTINUES: Self = Self(2);

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

/// A lexical token. Spans are non-empty UTF-8 byte ranges; a construct with
/// no bytes of its own (a value discarded into nothing) produces no token.
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

/// Lossless lexer output: the token stream plus the diagnostics the lexer
/// itself can state (broken literals, stray bytes, unclosed strings).
/// Concatenating token text always reconstructs the source byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lexed<'source> {
    source: &'source str,
    tokens: Vec<LexToken>,
    diagnostics: Vec<Diagnostic>,
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

    /// Lexical diagnostics only; structure diagnostics come from
    /// [`crate::parse`].
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
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

/// Lexes an EDN document.
#[must_use]
pub fn lex(source: &str) -> Lexed<'_> {
    let mut lexer = Lexer {
        source,
        bytes: source.as_bytes(),
        pos: 0,
        tokens: Vec::new(),
        diagnostics: Vec::new(),
    };
    lexer.run();
    Lexed {
        source,
        tokens: lexer.tokens,
        diagnostics: lexer.diagnostics,
    }
}

struct Lexer<'source> {
    source: &'source str,
    bytes: &'source [u8],
    pos: usize,
    tokens: Vec<LexToken>,
    diagnostics: Vec<Diagnostic>,
}

/// UTF-8 encoding of U+FEFF.
const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// EDN whitespace: spaces, tabs, line breaks, form feed — and commas, which
/// the spec parses as whitespace. Every member is ASCII, so a continuation
/// byte can never split a run mid-character.
const fn is_ws(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0C | b',')
}

/// Whether a byte may sit inside a symbol/keyword token. `:` `#` `@` are
/// interior-only (the dispatch and keyword paths own the leading positions);
/// `/` is included and validated afterwards (at most one, in the middle).
fn is_symbol_cont(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(
            c,
            '_' | '+'
                | '-'
                | '.'
                | '*'
                | '!'
                | '?'
                | '$'
                | '%'
                | '&'
                | '='
                | '<'
                | '>'
                | ':'
                | '#'
                | '@'
                | '/'
        )
}

/// The named character literals in use beyond `\c` and `\uNNNN`. The spec
/// names four; `formfeed` and `backspace` are what EDN readers accept.
const NAMED_CHARS: [&str; 6] = ["newline", "return", "space", "tab", "formfeed", "backspace"];

/// String escapes that stand alone: the spec's `\t \r \n \\ \"` plus the
/// C/Java `\b \f` and the `\uNNNN` / `\UNNNNNNNN` forms EDN readers accept.
fn is_plain_string_escape(c: char) -> bool {
    matches!(c, 'b' | 't' | 'n' | 'f' | 'r' | '"' | '\\')
}

/// What a symbol_shaped token text is: plain, namespace_qualified, or not a
/// legal symbol shape at all.
enum SymbolShape {
    Plain,
    Namespaced,
    Invalid,
}

/// Shared validation for symbol, keyword_name and tag texts, per the spec:
/// no leading digit (nor leading `/`, which is only legal alone — handled by
/// the caller), a leading `+ - .` needs a non_numeric second character, and
/// at most one interior `/` with both sides non_empty and the name not
/// starting with a digit.
fn symbol_shape(text: &str) -> SymbolShape {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return SymbolShape::Invalid;
    };
    if text == "/" {
        // A lone slash is a legal symbol (every reader's division fn); the
        // namespace rules only bind when there is more around it.
        return SymbolShape::Plain;
    }
    if first.is_numeric() || first == '/' {
        return SymbolShape::Invalid;
    }
    if matches!(first, '+' | '-' | '.') && chars.next().is_some_and(|c| c.is_numeric()) {
        return SymbolShape::Invalid;
    }
    let slashes = text.matches('/').count();
    match slashes {
        0 => SymbolShape::Plain,
        1 => {
            let slash = text.find('/').unwrap_or(0);
            let name = &text[slash + 1..];
            if slash == 0 || name.is_empty() || name.chars().next().is_some_and(|c| c.is_numeric())
            {
                SymbolShape::Invalid
            } else {
                SymbolShape::Namespaced
            }
        }
        _ => SymbolShape::Invalid,
    }
}

impl Lexer<'_> {
    fn run(&mut self) {
        self.lex_bom();
        while self.pos < self.bytes.len() {
            let byte = self.bytes[self.pos];
            if is_ws(byte) {
                self.lex_whitespace();
            } else if byte == b';' {
                self.lex_comment();
            } else if byte == b'"' {
                self.lex_string();
            } else if byte == b'#' {
                self.lex_hash();
            } else if byte == b'\\' {
                self.lex_character();
            } else if byte == b':' {
                self.lex_keyword();
            } else {
                self.lex_token_or_delimiter();
            }
        }
    }

    /// Every non-progressing shape is caught by the loop guard below: each
    /// branch consumes at least one byte, so `pos` strictly increases.
    fn lex_bom(&mut self) {
        if self.bytes.starts_with(&BOM) {
            self.push(SyntaxKind::Bom, 0, BOM.len(), false);
            self.pos = BOM.len();
        }
    }

    fn lex_whitespace(&mut self) {
        let start = self.pos;
        let mut end = start;
        while end < self.bytes.len() && is_ws(self.bytes[end]) {
            end += 1;
        }
        self.push(SyntaxKind::Whitespace, start, end, false);
        self.pos = end;
    }

    /// `;` through just before the next newline; a comment at EOF runs to the
    /// last byte.
    fn lex_comment(&mut self) {
        let start = self.pos;
        let mut end = start + 1;
        while end < self.bytes.len() && self.bytes[end] != b'\n' {
            end += 1;
        }
        self.push(SyntaxKind::Comment, start, end, false);
        self.pos = end;
    }

    fn lex_delimiter(&mut self, kind: SyntaxKind) {
        self.push(kind, self.pos, self.pos + 1, false);
        self.pos += 1;
    }

    /// Delimiters, symbol/number tokens, and one-character error tokens for
    /// bytes that start no EDN construct at all (a stray `|` or `§`).
    fn lex_token_or_delimiter(&mut self) {
        match self.bytes[self.pos] {
            b'(' => return self.lex_delimiter(SyntaxKind::ListOpen),
            b')' => return self.lex_delimiter(SyntaxKind::ListClose),
            b'[' => return self.lex_delimiter(SyntaxKind::VectorOpen),
            b']' => return self.lex_delimiter(SyntaxKind::VectorClose),
            b'{' => return self.lex_delimiter(SyntaxKind::MapOpen),
            b'}' => return self.lex_delimiter(SyntaxKind::MapClose),
            _ => {}
        }
        // Clojure reader macros that are not data: the dispatch byte and the
        // whole attached token are kept as one error run.
        if matches!(self.bytes[self.pos], b'@' | b'^' | b'\'' | b'`' | b'~') {
            self.lex_reader_macro();
            return;
        }
        let Some((first, first_len)) = self.char_at(self.pos) else {
            // Cannot happen for valid UTF-8 at a boundary, but never stall.
            self.push(SyntaxKind::Error, self.pos, self.pos + 1, true);
            self.add_diagnostic(DiagnosticKind::InvalidSymbol, self.pos, self.pos + 1);
            self.pos += 1;
            return;
        };
        if !is_symbol_cont(first) {
            let end = self.pos + first_len;
            self.push(SyntaxKind::Error, self.pos, end, true);
            self.add_diagnostic(DiagnosticKind::InvalidSymbol, self.pos, end);
            self.pos = end;
            return;
        }
        let start = self.pos;
        let end = self.scan_cont_run(self.pos);
        let run = &self.source[start..end];
        self.lex_symbol_or_number(start, end, run);
        self.pos = end;
    }

    /// A `@`, `^`, `'`, `` ` `` or `~` head with its attached token text: not
    /// EDN data, kept whole so the bytes stay in the stream.
    fn lex_reader_macro(&mut self) {
        let start = self.pos;
        let (_, head_len) = self.char_at(start).unwrap_or(('~', 1));
        let end = self.scan_cont_run(start + head_len);
        self.push(SyntaxKind::Error, start, end.max(start + head_len), true);
        self.add_diagnostic(
            DiagnosticKind::NonEdnConstruct,
            start,
            end.max(start + head_len),
        );
        self.pos = end.max(start + head_len);
    }

    /// Classify an already-scanned symbol/number run and emit its token.
    fn lex_symbol_or_number(&mut self, start: usize, end: usize, run: &str) {
        let bytes = run.as_bytes();
        let numeric_start = bytes.first().is_some_and(|b| b.is_ascii_digit())
            || bytes
                .first()
                .is_some_and(|b| matches!(b, b'+' | b'-' | b'.'))
                && bytes.get(1).is_some_and(|b| b.is_ascii_digit());
        if numeric_start {
            match classify_numeric(bytes) {
                Ok(kind) => self.push(kind, start, end, false),
                Err(kind) => {
                    self.push(SyntaxKind::Error, start, end, true);
                    self.add_diagnostic(kind, start, end);
                }
            }
            return;
        }
        match symbol_shape(run) {
            SymbolShape::Invalid => {
                self.push(SyntaxKind::Error, start, end, true);
                self.add_diagnostic(DiagnosticKind::InvalidSymbol, start, end);
            }
            SymbolShape::Plain => {
                let kind = match run {
                    "true" | "false" => SyntaxKind::BooleanLiteral,
                    "nil" => SyntaxKind::NilLiteral,
                    _ => SyntaxKind::Symbol,
                };
                self.push(kind, start, end, false);
            }
            SymbolShape::Namespaced => self.push(SyntaxKind::NamespacedSymbol, start, end, false),
        }
    }

    /// `:` + a symbol-shaped name. `::` shorthands are Clojure ns-aliases,
    /// not EDN; a bare `:` is a keyword with no name.
    fn lex_keyword(&mut self) {
        let start = self.pos;
        let end = self.scan_cont_run(start + 1);
        let name = &self.source[start + 1..end];
        if name.is_empty() {
            self.push(SyntaxKind::Error, start, end, true);
            self.add_diagnostic(DiagnosticKind::InvalidSymbol, start, end);
            self.pos = end;
            return;
        }
        if name.starts_with(':') {
            self.push(SyntaxKind::Error, start, end, true);
            self.add_diagnostic(DiagnosticKind::NonEdnConstruct, start, end);
            self.pos = end;
            return;
        }
        if name == "/" {
            self.push(SyntaxKind::Error, start, end, true);
            self.add_diagnostic(DiagnosticKind::InvalidSymbol, start, end);
            self.pos = end;
            return;
        }
        match symbol_shape(name) {
            SymbolShape::Invalid => {
                self.push(SyntaxKind::Error, start, end, true);
                self.add_diagnostic(DiagnosticKind::InvalidSymbol, start, end);
            }
            SymbolShape::Plain => self.push(SyntaxKind::Keyword, start, end, false),
            SymbolShape::Namespaced => self.push(SyntaxKind::NamespacedKeyword, start, end, false),
        }
        self.pos = end;
    }

    /// Everything that may follow `#`: set open, discard, `#:`, `##`, a tag,
    /// or nothing legal at all.
    fn lex_hash(&mut self) {
        let start = self.pos;
        match self.bytes.get(start + 1) {
            Some(b'{') => {
                self.push(SyntaxKind::SetOpen, start, start + 2, false);
                self.pos = start + 2;
            }
            Some(b'_') => {
                self.push(SyntaxKind::Discard, start, start + 2, false);
                self.pos = start + 2;
            }
            Some(b':') => self.lex_namespaced_map_prefix(),
            Some(b'#') => self.lex_special_number(),
            Some(&byte) => {
                let is_tag_head = if byte < 0x80 {
                    byte.is_ascii_alphabetic()
                } else {
                    let (c, _) = self.char_at(start + 1).unwrap_or((' ', 1));
                    c.is_alphabetic()
                };
                if is_tag_head {
                    self.lex_tag();
                    return;
                }
                // `#(`, `#'`, `#=`, `#&`, `#?`, `#"…`, `#1…`: a dispatch
                // character EDN never defines.
                if byte == b';' || is_ws(byte) {
                    self.push(SyntaxKind::Error, start, start + 1, true);
                    self.add_diagnostic(DiagnosticKind::IncompleteDispatch, start, start + 1);
                } else {
                    self.push(SyntaxKind::Error, start, start + 1, true);
                    self.add_diagnostic(DiagnosticKind::NonEdnConstruct, start, start + 1);
                }
                self.pos = start + 1;
            }
            None => {
                self.push(SyntaxKind::Error, start, start + 1, true);
                self.add_diagnostic(DiagnosticKind::IncompleteDispatch, start, start + 1);
                self.pos = start + 1;
            }
        }
    }

    /// `#` + symbol: `#inst`, `#uuid`, `#b`, or a user tag.
    fn lex_tag(&mut self) {
        let start = self.pos;
        let end = self.scan_cont_run(start + 1);
        let name = &self.source[start + 1..end];
        self.pos = end;
        match symbol_shape(name) {
            SymbolShape::Invalid => {
                self.push(SyntaxKind::Error, start, end, true);
                self.add_diagnostic(DiagnosticKind::InvalidSymbol, start, end);
            }
            SymbolShape::Plain | SymbolShape::Namespaced => {
                let kind = match name {
                    "inst" => SyntaxKind::InstantTag,
                    "uuid" => SyntaxKind::UuidTag,
                    "b" => SyntaxKind::ByteTag,
                    _ => SyntaxKind::Tag,
                };
                self.push(kind, start, end, false);
            }
        }
    }

    /// `#:` + symbol, the prefix of a namespaced map. `#::` is Clojure's
    /// auto_resolving form and not data.
    fn lex_namespaced_map_prefix(&mut self) {
        let start = self.pos;
        let end = self.scan_cont_run(start + 2);
        let name = &self.source[start + 2..end];
        self.pos = end;
        if name.is_empty() {
            self.push(SyntaxKind::Error, start, end, true);
            self.add_diagnostic(DiagnosticKind::InvalidSymbol, start, end);
            return;
        }
        if name.starts_with(':') {
            self.push(SyntaxKind::Error, start, end, true);
            self.add_diagnostic(DiagnosticKind::NonEdnConstruct, start, end);
            return;
        }
        if name == "/" || matches!(symbol_shape(name), SymbolShape::Invalid) {
            self.push(SyntaxKind::Error, start, end, true);
            self.add_diagnostic(DiagnosticKind::InvalidSymbol, start, end);
            return;
        }
        self.push(SyntaxKind::NamespacedMapPrefix, start, end, false);
    }

    /// `##Inf`, `##-Inf`, `##NaN` — the special numeric values of the EDN
    /// reference grammar. Anything else after `##` is not a number.
    fn lex_special_number(&mut self) {
        let start = self.pos;
        let mut end = start + 2;
        if self.bytes.get(end) == Some(&b'-') {
            end += 1;
        }
        while let Some((c, len)) = self.char_at(end) {
            if c.is_alphabetic() {
                end += len;
            } else {
                break;
            }
        }
        self.pos = end;
        if end == start + 2 || end == start + 3 && self.bytes[start + 2] == b'-' {
            self.push(SyntaxKind::Error, start, end, true);
            self.add_diagnostic(DiagnosticKind::InvalidNumber, start, end);
            return;
        }
        match &self.source[start + 2..end] {
            "Inf" | "-Inf" | "NaN" => self.push(SyntaxKind::SpecialNumber, start, end, false),
            _ => {
                self.push(SyntaxKind::Error, start, end, true);
                self.add_diagnostic(DiagnosticKind::InvalidNumber, start, end);
            }
        }
    }

    /// A character literal: `\c`, `\newline`, `\u00e9`, `\(`. The backslash
    /// may never be followed by whitespace, and a multi_character name is
    /// only legal when it is a named character or `u` + four hex digits.
    fn lex_character(&mut self) {
        let start = self.pos;
        let Some((first, first_len)) = self.char_at(start + 1) else {
            self.push(SyntaxKind::Error, start, start + 1, true);
            self.add_diagnostic(DiagnosticKind::MalformedChar, start, start + 1);
            self.pos = start + 1;
            return;
        };
        if first.is_ascii_whitespace() || first == ',' {
            self.push(SyntaxKind::Error, start, start + 1, true);
            self.add_diagnostic(DiagnosticKind::MalformedChar, start, start + 1);
            self.pos = start + 1;
            return;
        }
        if first.is_ascii_alphanumeric() {
            let mut end = start + 1;
            while let Some((c, len)) = self.char_at(end) {
                if c.is_ascii_alphanumeric() {
                    end += len;
                } else {
                    break;
                }
            }
            let name = &self.source[start + 1..end];
            let valid = name.len() == 1
                || NAMED_CHARS.contains(&name)
                || (name.starts_with('u')
                    && name.len() == 5
                    && name[1..].bytes().all(|b| b.is_ascii_hexdigit()));
            if valid {
                self.push(SyntaxKind::Character, start, end, false);
            } else {
                self.push(SyntaxKind::Error, start, end, true);
                self.add_diagnostic(DiagnosticKind::MalformedChar, start, end);
            }
            self.pos = end;
            return;
        }
        // Any other single character stands for itself: `\(`, `\\`, `\;`, `\é`.
        let end = start + 1 + first_len;
        self.push(SyntaxKind::Character, start, end, false);
        self.pos = end;
    }

    /// A string: literal runs interleaved with escape sequences, the
    /// surrounding quotes carried by the runs so every byte stays covered.
    /// An unknown escape or a short `\u` is an error_flagged escape token; an
    /// unterminated string flags its trailing run and reports the whole
    /// region.
    fn lex_string(&mut self) {
        let open = self.pos;
        let region_head = self.tokens.len();
        let mut run_start = open;
        let mut cursor = open + 1;
        let mut closed = false;
        while cursor < self.bytes.len() {
            let byte = self.bytes[cursor];
            if byte == b'"' {
                self.push(SyntaxKind::String, run_start, cursor + 1, false);
                closed = true;
                cursor += 1;
                break;
            }
            if byte == b'\\' {
                self.push(SyntaxKind::String, run_start, cursor, false);
                cursor = self.lex_string_escape(cursor);
                run_start = cursor;
                continue;
            }
            cursor += 1;
        }
        if !closed {
            self.push(SyntaxKind::String, run_start, self.bytes.len(), true);
            self.add_diagnostic(DiagnosticKind::UnterminatedString, open, self.bytes.len());
            self.pos = self.bytes.len();
            self.mark_region_continuations(region_head);
            return;
        }
        self.pos = cursor;
        self.mark_region_continuations(region_head);
    }

    /// Everything after a string region's head token is part of that same
    /// element: escapes and tail runs carry [`TokenFlags::CONTINUES`] so the
    /// structure pass never mistakes one string for several forms.
    fn mark_region_continuations(&mut self, region_head: usize) {
        for token in &mut self.tokens[region_head + 1..] {
            token.flags = token.flags.with(TokenFlags::CONTINUES);
        }
    }

    /// Consume the escape starting at the backslash and return the cursor
    /// after it. Progress is guaranteed: at least the backslash.
    fn lex_string_escape(&mut self, backslash: usize) -> usize {
        let Some((c, c_len)) = self.char_at(backslash + 1) else {
            // A lone trailing backslash: an error-flagged escape of two
            // bytes when a partner exists, else just the backslash.
            let end = backslash + 1;
            self.push(SyntaxKind::StringEscape, backslash, end, true);
            self.add_diagnostic(DiagnosticKind::InvalidEscape, backslash, end);
            return end;
        };
        if is_plain_string_escape(c) {
            let end = backslash + 1 + c_len;
            self.push(SyntaxKind::StringEscape, backslash, end, false);
            return end;
        }
        if c == 'u' || c == 'U' {
            let want = if c == 'u' { 4 } else { 8 };
            let mut cursor = backslash + 2;
            let mut digits = 0;
            while digits < want && self.bytes.get(cursor) == Some(&b'0') {
                cursor += 1;
                digits += 1;
            }
            while digits < want
                && self
                    .bytes
                    .get(cursor)
                    .is_some_and(|b| b.is_ascii_hexdigit())
            {
                cursor += 1;
                digits += 1;
            }
            // A `0` was greedily taken as a digit; a short run after it is
            // still malformed, and the bytes belong to the escape token.
            let end = if digits == want {
                cursor
            } else {
                backslash + 2
            };
            self.push(SyntaxKind::StringEscape, backslash, end, digits != want);
            if digits != want {
                self.add_diagnostic(DiagnosticKind::InvalidEscape, backslash, end);
            }
            return end;
        }
        // Unknown escape letter (including a raw newline after the `\`):
        // both bytes stay in the string, flagged.
        let end = backslash + 1 + c_len;
        self.push(SyntaxKind::StringEscape, backslash, end, true);
        self.add_diagnostic(DiagnosticKind::InvalidEscape, backslash, end);
        end
    }

    // ─── scanning helpers ────────────────────────────────────────────────────

    fn char_at(&self, at: usize) -> Option<(char, usize)> {
        let c = self.source.get(at..)?.chars().next()?;
        Some((c, c.len_utf8()))
    }

    /// End of the maximal symbol-continuation run starting at `start`.
    fn scan_cont_run(&self, start: usize) -> usize {
        let mut end = start;
        while let Some((c, len)) = self.char_at(end) {
            if !is_symbol_cont(c) {
                break;
            }
            end += len;
        }
        end
    }

    // ─── token output ────────────────────────────────────────────────────────

    /// Emits a token; zero-width spans are skipped so the stream never
    /// carries an empty token.
    fn push(&mut self, kind: SyntaxKind, start: usize, end: usize, error: bool) {
        if end <= start {
            return;
        }
        let flags = if error {
            TokenFlags::HAS_ERROR
        } else {
            TokenFlags::EMPTY
        };
        self.tokens.push(LexToken {
            kind,
            span: Span::new(start, end),
            flags,
        });
    }

    fn add_diagnostic(&mut self, kind: DiagnosticKind, start: usize, end: usize) {
        if end <= start {
            return;
        }
        self.diagnostics
            .push(kind.to_diagnostic(Span::new(start, end)));
    }
}

/// Classify a numeric-looking byte run against the EDN number grammar:
/// `[+-]?digits`, optional `N` (bigint) or `M` (decimal), fractions,
/// optional exponent, and radix form `[+-]?radix r digits [N]`.
/// `Err` names the diagnostic for a token that looks numeric but is none;
/// such a run can also never be a symbol (symbols start non-numeric), so the
/// caller flags the whole run.
fn classify_numeric(bytes: &[u8]) -> Result<SyntaxKind, DiagnosticKind> {
    let sign_len = usize::from(matches!(bytes.first(), Some(b'+') | Some(b'-')));
    let mut i = sign_len;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let digits_end = i;

    // Radix form: at least one decimal radix digit, then `r`.
    if digits_end > sign_len && matches!(bytes.get(digits_end), Some(b'r') | Some(b'R')) {
        let mut radix: u32 = 0;
        for &digit in &bytes[sign_len..digits_end] {
            radix = radix.saturating_mul(10) + u32::from(digit - b'0');
        }
        if !(2..=36).contains(&radix) {
            return Err(DiagnosticKind::InvalidRadix);
        }
        let mut j = digits_end + 1;
        while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
            j += 1;
        }
        let mut payload = &bytes[digits_end + 1..j];
        // A trailing `N` is the arbitrary-precision suffix when anything is
        // left to be a digit; otherwise it is itself a radix-33 digit.
        if payload.last() == Some(&b'N') && payload.len() > 1 {
            payload = &payload[..payload.len() - 1];
        }
        if payload.is_empty() {
            return Err(DiagnosticKind::InvalidRadixDigit);
        }
        for &byte in payload {
            let value = radix_digit_value(byte).ok_or(DiagnosticKind::InvalidNumber)?;
            if value >= radix {
                return Err(DiagnosticKind::InvalidRadixDigit);
            }
        }
        return if j == bytes.len() {
            Ok(SyntaxKind::RadixInteger)
        } else {
            Err(DiagnosticKind::InvalidNumber)
        };
    }

    // Plain int part with the spec's leading-zero rule.
    let int_text = &bytes[sign_len..digits_end];
    let int_text = core::str::from_utf8(int_text).unwrap_or_default();
    i = digits_end;
    if int_text.is_empty() || (int_text.len() > 1 && int_text.starts_with('0')) {
        return Err(DiagnosticKind::InvalidNumber);
    }
    if i == bytes.len() {
        return Ok(SyntaxKind::Integer);
    }
    match bytes[i] {
        b'/' => {
            // Ratio: two integer grammars and nothing else.
            i += 1;
            if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
                i += 1;
            }
            let den_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let den = &bytes[den_start..i];
            let Ok(den_text) = std::str::from_utf8(den) else {
                return Err(DiagnosticKind::InvalidNumber);
            };
            if den_text.is_empty()
                || i != bytes.len()
                || (den_text.len() > 1 && den_text.starts_with('0'))
            {
                return Err(DiagnosticKind::InvalidNumber);
            }
            Ok(SyntaxKind::Ratio)
        }
        b'M' if i + 1 == bytes.len() => Ok(SyntaxKind::Decimal),
        b'N' if i + 1 == bytes.len() => Ok(SyntaxKind::BigInteger),
        b'.' => {
            i += 1;
            let frac_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == frac_start {
                return Err(DiagnosticKind::InvalidNumber);
            }
            if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
                i += 1;
                if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
                    i += 1;
                }
                let exp_start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i == exp_start {
                    return Err(DiagnosticKind::InvalidNumber);
                }
            }
            float_tail(bytes, i, SyntaxKind::Float)
        }
        b'e' | b'E' => {
            i += 1;
            if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
                i += 1;
            }
            let exp_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == exp_start {
                return Err(DiagnosticKind::InvalidNumber);
            }
            float_tail(bytes, i, SyntaxKind::Float)
        }
        _ => Err(DiagnosticKind::InvalidNumber),
    }
}

/// After a fraction or exponent, only a terminal `M` may follow.
fn float_tail(bytes: &[u8], i: usize, kind: SyntaxKind) -> Result<SyntaxKind, DiagnosticKind> {
    if i == bytes.len() {
        return Ok(kind);
    }
    if bytes[i] == b'M' && i + 1 == bytes.len() {
        return Ok(SyntaxKind::Decimal);
    }
    Err(DiagnosticKind::InvalidNumber)
}

/// Value of a radix digit (case-insensitive), or `None` if not a digit at all.
pub(crate) fn radix_digit_value(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => u32::from(byte - b'0').into(),
        b'a'..=b'z' => u32::from(byte - b'a' + 10).into(),
        b'A'..=b'Z' => u32::from(byte - b'A' + 10).into(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Documents the shapes real EDN writers produce: every one of these
    /// must lex clean *and* losslessly.
    const CLEAN_CORPUS: &[&str] = &[
        "42",
        "-7 +5",
        "0 -0 +0",
        "3/4",
        "2r1010 16rFF 8r777 -2r1",
        "1000N",
        "3.25 6.022e23 1E5 -0.5",
        "3.25M 7M 1e5M",
        "##Inf ##-Inf ##NaN",
        "nil true false",
        "sym another_sym yet_another .. + * / foo/bar clojure.string/join",
        ":kw :ns/kw :a:b",
        "\\c \\newline \\space \\u00e9 \\\\ \\( \\;",
        "\"a string with \\\" escape and \\n\"",
        "()",
        "[] {} #{}",
        "{:a 1 :b [2 3]} #{1 2 3}",
        "#inst \"1985-04-12T23:20:50.52Z\"",
        "#uuid \"f81d4fae-7dec-11d0-a765-00a0c91e6bf6\"",
        "#b \"\\u0000\\u007f\"",
        "#myco/Person {:first \"Lucy\"}",
        "#_[:scratch 1]",
        "#:app{:level :info}",
        "; a comment at eof",
        "a ; trailing\nb",
        "one,,two; x\nthree",
        "\u{FEFF}nil",
        "ünïcode/sym-é",
    ];

    /// Malformed or merely unusual input: still byte_for_byte recoverable.
    const BROKEN_CORPUS: &[&str] = &[
        "",
        "  ",
        "\n",
        "#",
        "#_",
        "#_ ",
        "#{",
        "#b",
        "#:",
        "#::app{:a 1}",
        "#(inc 1)",
        "#'var",
        "#=1",
        "@form",
        "^:meta {}",
        "'quoted",
        "`tilded",
        "~unquoted",
        "~@spliced",
        "::nskw",
        ":",
        ":/",
        "1e",
        "2r",
        "1r11",
        "2r1020",
        "01",
        "1.2.3",
        "1abc",
        ".5",
        "##",
        "##nan",
        "\\",
        "\\u12",
        "\\uZZZZ",
        "\\nbsp",
        "\\ newline",
        "\"unterminated",
        "\"bad \\q escape",
        "\"short \\u12\"",
        "[1 2",
        "{:a]",
        "]",
        ")",
        "a//b",
        "/a",
        "a/",
        "a/1b",
        "foo|bar",
        "3/4/5",
        "99999999999999999999999999999999999999999999999999999N",
        "\u{FEFF}",
        "\"string\"\u{FEFF}nil",
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
    fn clean_corpus_has_no_lexical_diagnostics() {
        for source in CLEAN_CORPUS {
            let lexed = lex(source);
            assert!(
                lexed.diagnostics().is_empty() && !lexed.has_errors(),
                "{source:?}: {:?}",
                lexed.diagnostics()
            );
        }
    }

    #[test]
    fn every_truncation_rebuilds_its_prefix() {
        let sample = concat!(
            "{:a [1 #_2 (3.5M 2r10 :b/c #inst \"1985-04-12\" #\"re\")],\n",
            " :s #\"x \\\"y\\u00zz ; not-a-comment \\ \n",
            " #:app{:k 'q} #{\\space ##Inf nil} ; tail\n",
        );
        for cut in 0..=sample.len() {
            assert_lossless(&sample[..cut]);
        }
    }

    #[test]
    fn numbers_are_classified_by_their_edn_grammar() {
        let tokens = kinds_and_text("42 -7 +5 1000N 2r1010 16rFF 3/4 1.5 6.022e23 7M 1e5M ##Inf");
        let numeric: Vec<SyntaxKind> = tokens
            .into_iter()
            .filter(|(kind, _)| !matches!(kind, SyntaxKind::Whitespace))
            .map(|(kind, _)| kind)
            .collect();
        assert_eq!(
            numeric,
            vec![
                SyntaxKind::Integer,
                SyntaxKind::Integer,
                SyntaxKind::Integer,
                SyntaxKind::BigInteger,
                SyntaxKind::RadixInteger,
                SyntaxKind::RadixInteger,
                SyntaxKind::Ratio,
                SyntaxKind::Float,
                SyntaxKind::Float,
                SyntaxKind::Decimal,
                SyntaxKind::Decimal,
                SyntaxKind::SpecialNumber,
            ]
        );
    }

    #[test]
    fn numeric_mistakes_are_flagged_with_codes() {
        let codes: Vec<&str> = lex("1e 2r 1r11 2r1020 01 1abc .5 ##nan 1/2/3")
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert_eq!(
            codes,
            vec![
                "invalid-number",
                "invalid-radix-digit",
                "invalid-radix",
                "invalid-radix-digit",
                "invalid-number",
                "invalid-number",
                "invalid-number",
                "invalid-number",
                "invalid-number",
            ]
        );
    }

    #[test]
    fn symbols_keywords_and_literals_are_named_exactly() {
        assert_eq!(
            kinds_and_text("nil true false user my.ns/foo clojure.string/join / :kw :a/b"),
            vec![
                (SyntaxKind::NilLiteral, "nil"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::BooleanLiteral, "true"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::BooleanLiteral, "false"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Symbol, "user"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::NamespacedSymbol, "my.ns/foo"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::NamespacedSymbol, "clojure.string/join"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Symbol, "/"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Keyword, ":kw"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::NamespacedKeyword, ":a/b"),
            ]
        );
        // Interior `: # @` are legal symbol characters, leading ones are not.
        assert_eq!(kinds("a:b"), vec![SyntaxKind::Symbol]);
        assert_eq!(kinds("a#b"), vec![SyntaxKind::Symbol]);
        assert_eq!(kinds("a@b"), vec![SyntaxKind::Symbol]);
        assert_eq!(
            kinds("+ . .. +x -x"),
            vec![
                SyntaxKind::Symbol,
                SyntaxKind::Whitespace,
                SyntaxKind::Symbol,
                SyntaxKind::Whitespace,
                SyntaxKind::Symbol,
                SyntaxKind::Whitespace,
                SyntaxKind::Symbol,
                SyntaxKind::Whitespace,
                SyntaxKind::Symbol,
            ]
        );
        assert_eq!(kinds("+5x"), vec![SyntaxKind::Error]);
    }

    #[test]
    fn underscores_are_legal_in_symbols_and_keywords() {
        assert_eq!(
            kinds_and_text("foo_bar _baz :some_key :ns/some_key"),
            vec![
                (SyntaxKind::Symbol, "foo_bar"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Symbol, "_baz"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Keyword, ":some_key"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::NamespacedKeyword, ":ns/some_key"),
            ]
        );
        // An underscore may not group digits: `1_000` is not an EDN number.
        assert_eq!(kinds("1_000"), vec![SyntaxKind::Error]);
        let broken = lex("1_000");
        assert_eq!(
            broken
                .diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            vec!["invalid-number"]
        );
    }

    #[test]
    fn set_open_and_discard_are_two_byte_tokens() {
        assert_eq!(
            kinds_and_text("#{} #_x"),
            vec![
                (SyntaxKind::SetOpen, "#{"),
                (SyntaxKind::MapClose, "}"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Discard, "#_"),
                (SyntaxKind::Symbol, "x"),
            ]
        );
    }

    #[test]
    fn tags_are_named_by_their_edn_role() {
        assert_eq!(
            kinds("#inst #uuid #b #myco/Person #weird"),
            vec![
                SyntaxKind::InstantTag,
                SyntaxKind::Whitespace,
                SyntaxKind::UuidTag,
                SyntaxKind::Whitespace,
                SyntaxKind::ByteTag,
                SyntaxKind::Whitespace,
                SyntaxKind::Tag,
                SyntaxKind::Whitespace,
                SyntaxKind::Tag,
            ]
        );
        assert_eq!(
            kinds_and_text("#:app{:a 1}"),
            vec![
                (SyntaxKind::NamespacedMapPrefix, "#:app"),
                (SyntaxKind::MapOpen, "{"),
                (SyntaxKind::Keyword, ":a"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Integer, "1"),
                (SyntaxKind::MapClose, "}"),
            ]
        );
        assert_eq!(
            kinds("#:a/b{}"),
            vec![
                SyntaxKind::NamespacedMapPrefix,
                SyntaxKind::MapOpen,
                SyntaxKind::MapClose,
            ]
        );
    }

    #[test]
    fn character_literals_take_one_or_a_whole_name() {
        assert_eq!(
            kinds_and_text(r"\c \newline \space \u00e9 \\ \; \("),
            vec![
                (SyntaxKind::Character, r"\c"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Character, r"\newline"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Character, r"\space"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Character, r"\u00e9"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Character, r"\\"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Character, r"\;"),
                (SyntaxKind::Whitespace, " "),
                (SyntaxKind::Character, r"\("),
            ]
        );
        // A `\` may not be followed by whitespace, and unknown long names
        // are malformed, but every byte stays accounted for.
        let broken = lex(r"\ \nbsp \u12 \");
        assert!(broken.has_errors());
        assert_eq!(broken.diagnostics().len(), 4);
        assert!(
            broken
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.code == "malformed-char")
        );
    }

    #[test]
    fn strings_carve_out_their_escapes() {
        assert_eq!(
            kinds_and_text("\"say \\\"hi\\\" \u{A0}\nnext\""),
            vec![
                (SyntaxKind::String, "\"say "),
                (SyntaxKind::StringEscape, "\\\""),
                (SyntaxKind::String, "hi"),
                (SyntaxKind::StringEscape, "\\\""),
                (SyntaxKind::String, " \u{A0}\nnext\""),
            ]
        );
        assert_eq!(kinds("\"\""), vec![SyntaxKind::String]);
    }

    #[test]
    fn bad_string_escapes_are_flagged_not_dropped() {
        let lexed = lex("\"a \\q b \\u12\" tail");
        let codes: Vec<&str> = lexed
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert_eq!(codes, vec!["invalid-escape", "invalid-escape"]);
        assert_eq!(lexed.joined(), "\"a \\q b \\u12\" tail");
        // A backslash at end of input keeps its byte and reports.
        let trailing = lex("\"x\\");
        assert_eq!(trailing.diagnostics().len(), 2); // invalid_escape + unterminated_string
    }

    #[test]
    fn unterminated_string_flags_its_region() {
        let lexed = assert_lossless("\"oops\nmore");
        assert!(lexed.has_errors());
        let codes: Vec<&str> = lexed
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert_eq!(codes, vec!["unterminated-string"]);
        assert_eq!(
            lexed.diagnostics()[0].span,
            Span::new(0, 10),
            "the whole region, open quote included"
        );
        assert!(
            lexed
                .tokens()
                .iter()
                .all(|token| token.kind == SyntaxKind::String)
        );
    }

    #[test]
    fn reader_macros_outside_edn_are_kept_as_errors() {
        for (source, expected) in [
            ("@form", 1),
            ("^:meta", 1),
            ("'q", 1),
            ("`t", 1),
            ("~u", 1),
            ("~@sp", 1),
            ("#(f)", 1),
            ("#'v", 2),
            ("#=1", 1),
            ("::ns", 1),
        ] {
            let lexed = assert_lossless(source);
            let codes: Vec<&str> = lexed
                .diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect();
            assert_eq!(codes, vec!["non-edn-construct"; expected], "{source:?}");
            assert_eq!(
                lexed
                    .tokens()
                    .iter()
                    .filter(|token| token.kind == SyntaxKind::Error)
                    .count(),
                expected,
                "{source:?}"
            );
        }
        // The dispatch byte stays; `#(` still opens its list for recovery.
        assert_eq!(
            kinds("#(inc 1)"),
            vec![
                SyntaxKind::Error,
                SyntaxKind::ListOpen,
                SyntaxKind::Symbol,
                SyntaxKind::Whitespace,
                SyntaxKind::Integer,
                SyntaxKind::ListClose,
            ]
        );
    }

    #[test]
    fn lone_hash_and_its_stalks_report_without_stalling() {
        assert_eq!(
            lex("#").diagnostics()[0].code,
            "incomplete-dispatch",
            "lone hash at eof"
        );
        assert_eq!(
            lex("# ").diagnostics()[0].code,
            "incomplete-dispatch",
            "hash before whitespace"
        );
        assert_eq!(
            lex("#_")
                .tokens()
                .iter()
                .map(|t| t.kind)
                .collect::<Vec<_>>(),
            vec![SyntaxKind::Discard],
            "lone discard is a well-formed token; only structure can blame it"
        );
        assert_eq!(
            lex("#b")
                .tokens()
                .iter()
                .map(|t| t.kind)
                .collect::<Vec<_>>(),
            vec![SyntaxKind::ByteTag],
            "the tag stands; its missing payload is a structure diagnostic"
        );
        assert_eq!(
            kinds("#{"),
            vec![SyntaxKind::SetOpen],
            "the set delimiter opened; EOF is structure's business"
        );
        assert_eq!(lex("#:").diagnostics()[0].code, "invalid-symbol");
    }

    #[test]
    fn comments_and_commas_never_swallow_the_next_form() {
        assert_eq!(
            kinds_and_text("a;c\n,b,"),
            vec![
                (SyntaxKind::Symbol, "a"),
                (SyntaxKind::Comment, ";c"),
                (SyntaxKind::Whitespace, "\n,"),
                (SyntaxKind::Symbol, "b"),
                (SyntaxKind::Whitespace, ","),
            ]
        );
        // A comment at EOF without a newline is one complete token.
        assert_eq!(kinds("; tail"), vec![SyntaxKind::Comment]);
    }

    #[test]
    fn bom_is_a_single_leading_token_and_nothing_else() {
        let lexed = assert_lossless("\u{FEFF}nil");
        assert_eq!(lexed.tokens()[0].kind, SyntaxKind::Bom);
        assert_eq!(lexed.tokens()[0].span, Span::new(0, 3));
        assert_eq!(lexed.tokens()[1].kind, SyntaxKind::NilLiteral);
        assert_eq!(kinds("\u{FEFF}"), vec![SyntaxKind::Bom]);
        // Away from position zero the BOM bytes are a stray character.
        let mid = assert_lossless("nil \u{FEFF}nil");
        assert!(
            mid.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == "invalid-symbol")
        );
    }

    #[test]
    fn empty_and_whitespace_only_input_lex_to_little() {
        assert!(assert_lossless("").tokens().is_empty());
        assert_eq!(assert_lossless(" ").tokens().len(), 1);
        assert_eq!(assert_lossless("\n\n").tokens().len(), 1);
    }
}
