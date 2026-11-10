//! Full multi-language engine kit: lossless lex → recovering parse → AST → semantic.
//!
//! Not a substitute for a language-spec-complete compiler front-end. It provides a
//! **uniform full pipeline** (LEX|PARSE|SEMANTIC) for editor tooling across
//! wave languages, with dialect profiles. The lexer recovers and reports what it
//! can prove (unbalanced delimiters, unclosed literals); the parser is silent by
//! design and only builds the AST that drives semantic kinds. `VALIDATE` is not
//! advertised — see [`FULL_ENGINE_CAPS`].

use crate::{
    Diagnostic, HostAnalysisOptions, HostDiagnostic, HostError, HostLanguage, HostSpan, HostToken,
    HostTokenization, LanguageDescriptor, Span, scan::match_common_operator, verify_lossless_spans,
};
use std::borrow::Cow;

// ─── Syntax tokens ───────────────────────────────────────────────────────────

/// Exact syntax kinds for the shared full pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SyntaxKind {
    Whitespace,
    LineComment,
    BlockComment,
    Keyword,
    TypeIdent,
    Identifier,
    StringLit,
    NumberLit,
    Punctuation,
    Operator,
    Error,
}

impl SyntaxKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Whitespace => "whitespace",
            Self::LineComment => "comment",
            Self::BlockComment => "comment",
            Self::Keyword => "keyword",
            Self::TypeIdent => "type",
            Self::Identifier => "identifier",
            Self::StringLit => "string",
            Self::NumberLit => "number",
            Self::Punctuation => "punctuation",
            Self::Operator => "operator",
            Self::Error => "error",
        }
    }

    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::Whitespace | Self::LineComment | Self::BlockComment
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LexToken {
    pub kind: SyntaxKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lexed {
    pub tokens: Vec<LexToken>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Lexed {
    #[must_use]
    pub fn is_lossless(&self, source: &str) -> bool {
        verify_lossless_spans(source, self.tokens.iter().map(|t| t.span)).is_ok()
    }
}

// ─── Profile & O(1) Keyword Lookup ───────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct FullProfile {
    pub keywords: &'static [&'static str],
    pub types: &'static [&'static str],
    pub line_comment: Option<&'static str>,
    pub block_comment: Option<(&'static str, &'static str)>,
    pub hash_line_comment: bool,
    /// Allow `$` in identifiers.
    pub dollar_ident: bool,
    /// Treat paired backticks as an opaque quoted span (SQL identifiers, shell commands).
    pub backtick_strings: bool,
    /// Python-style triple quotes.
    pub triple_strings: bool,
    /// `#` comments that are not only at BOL (shell/python already hash_line).
    pub soft_indent_blocks: bool,
}

impl FullProfile {
    pub const DEFAULT: Self = Self {
        keywords: &[],
        types: &[],
        line_comment: Some("//"),
        block_comment: Some(("/*", "*/")),
        hash_line_comment: false,
        dollar_ident: false,
        backtick_strings: false,
        triple_strings: false,
        soft_indent_blocks: false,
    };
}

impl Default for FullProfile {
    fn default() -> Self {
        Self::DEFAULT
    }
}

const LOOKUP_CAP: usize = 512;
const LOOKUP_MASK: usize = LOOKUP_CAP - 1;
const LOOKUP_MAX_ITEMS: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LookupSlot {
    kind: SyntaxKind,
    idx: u8,
}

impl LookupSlot {
    const EMPTY: Self = Self {
        kind: SyntaxKind::Whitespace,
        idx: 0,
    };
}

#[inline]
const fn hash_ascii_ci(bytes: &[u8]) -> u32 {
    let mut h = 0x811c_9dc5u32;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i].to_ascii_lowercase() as u32;
        h = h.wrapping_mul(0x0100_0193);
        i += 1;
    }
    h
}

#[inline]
const fn eq_ascii_ci(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i].to_ascii_lowercase() != b[i].to_ascii_lowercase() {
            return false;
        }
        i += 1;
    }
    true
}

/// Compile-time open-addressed hash table for $O(1)$ case-insensitive keyword and type lookup.
#[derive(Debug, Clone, Copy)]
pub struct KeywordLookup {
    slots: [LookupSlot; LOOKUP_CAP],
    len_mask: u64,
    complete: bool,
}

impl KeywordLookup {
    #[must_use]
    pub const fn from_profile(profile: &FullProfile) -> Self {
        let mut slots = [LookupSlot::EMPTY; LOOKUP_CAP];
        let mut len_mask = 0u64;
        if profile.keywords.len() > LOOKUP_MAX_ITEMS
            || profile.types.len() > LOOKUP_MAX_ITEMS
            || profile.keywords.len() + profile.types.len() > 384
        {
            return Self {
                slots,
                len_mask: u64::MAX,
                complete: false,
            };
        }

        let mut i = 0;
        while i < profile.keywords.len() {
            let kw = profile.keywords[i].as_bytes();
            let bit = if kw.len() < 64 { kw.len() } else { 63 };
            len_mask |= 1u64 << bit;
            let h = hash_ascii_ci(kw) as usize;
            let mut step = 0;
            while step < LOOKUP_CAP {
                let idx = (h + step) & LOOKUP_MASK;
                match slots[idx].kind {
                    SyntaxKind::Whitespace => {
                        slots[idx] = LookupSlot {
                            kind: SyntaxKind::Keyword,
                            idx: i as u8,
                        };
                        break;
                    }
                    SyntaxKind::Keyword => {
                        let existing = profile.keywords[slots[idx].idx as usize].as_bytes();
                        if eq_ascii_ci(existing, kw) {
                            break;
                        }
                    }
                    _ => {}
                }
                step += 1;
            }
            i += 1;
        }

        let mut j = 0;
        while j < profile.types.len() {
            let ty = profile.types[j].as_bytes();
            let bit = if ty.len() < 64 { ty.len() } else { 63 };
            len_mask |= 1u64 << bit;
            let h = hash_ascii_ci(ty) as usize;
            let mut step = 0;
            while step < LOOKUP_CAP {
                let idx = (h + step) & LOOKUP_MASK;
                match slots[idx].kind {
                    SyntaxKind::Whitespace => {
                        slots[idx] = LookupSlot {
                            kind: SyntaxKind::TypeIdent,
                            idx: j as u8,
                        };
                        break;
                    }
                    SyntaxKind::Keyword => {
                        let existing = profile.keywords[slots[idx].idx as usize].as_bytes();
                        if eq_ascii_ci(existing, ty) {
                            break;
                        }
                    }
                    SyntaxKind::TypeIdent => {
                        let existing = profile.types[slots[idx].idx as usize].as_bytes();
                        if eq_ascii_ci(existing, ty) {
                            break;
                        }
                    }
                    _ => {}
                }
                step += 1;
            }
            j += 1;
        }

        Self {
            slots,
            len_mask,
            complete: true,
        }
    }

    #[inline]
    #[must_use]
    pub fn classify(&self, profile: &FullProfile, ident: &str) -> SyntaxKind {
        let len_bit = 1u64 << ident.len().min(63);
        if (self.len_mask & len_bit) == 0 {
            return SyntaxKind::Identifier;
        }
        if !self.complete {
            return classify_linear(profile, ident);
        }
        let h = hash_ascii_ci(ident.as_bytes()) as usize;
        let mut step = 0;
        while step < LOOKUP_CAP {
            let slot = self.slots[(h + step) & LOOKUP_MASK];
            match slot.kind {
                SyntaxKind::Whitespace => return SyntaxKind::Identifier,
                SyntaxKind::Keyword => {
                    if profile.keywords[slot.idx as usize].eq_ignore_ascii_case(ident) {
                        return SyntaxKind::Keyword;
                    }
                }
                SyntaxKind::TypeIdent => {
                    if profile.types[slot.idx as usize].eq_ignore_ascii_case(ident) {
                        return SyntaxKind::TypeIdent;
                    }
                }
                _ => return SyntaxKind::Identifier,
            }
            step += 1;
        }
        SyntaxKind::Identifier
    }
}

// ─── AST ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module<'s> {
    pub span: Span,
    pub items: Vec<Item<'s>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item<'s> {
    Function {
        span: Span,
        name: Option<&'s str>,
        name_span: Option<Span>,
        body: Block<'s>,
    },
    Class {
        span: Span,
        name: Option<&'s str>,
        name_span: Option<Span>,
        body: Block<'s>,
    },
    Statement(Stmt<'s>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block<'s> {
    pub span: Span,
    pub stmts: Vec<Stmt<'s>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt<'s> {
    Expr {
        span: Span,
        expr: Expr<'s>,
    },
    Return {
        span: Span,
        value: Option<Expr<'s>>,
    },
    If {
        span: Span,
        cond: Option<Expr<'s>>,
        then_block: Block<'s>,
        else_block: Option<Block<'s>>,
    },
    While {
        span: Span,
        cond: Option<Expr<'s>>,
        body: Block<'s>,
    },
    For {
        span: Span,
        body: Block<'s>,
    },
    Declaration {
        span: Span,
        name: Option<&'s str>,
        name_span: Option<Span>,
        value: Option<Expr<'s>>,
    },
    Error {
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr<'s> {
    Ident {
        span: Span,
        name: &'s str,
    },
    Literal {
        span: Span,
        kind: LitKind,
        text: &'s str,
    },
    Call {
        span: Span,
        callee: Box<Expr<'s>>,
        args: Vec<Expr<'s>>,
    },
    Binary {
        span: Span,
        left: Box<Expr<'s>>,
        op_span: Span,
        right: Box<Expr<'s>>,
    },
    Unary {
        span: Span,
        op_span: Span,
        expr: Box<Expr<'s>>,
    },
    Member {
        span: Span,
        object: Box<Expr<'s>>,
        field: Option<&'s str>,
        field_span: Option<Span>,
    },
    Index {
        span: Span,
        object: Box<Expr<'s>>,
        index: Box<Expr<'s>>,
    },
    Paren {
        span: Span,
        expr: Box<Expr<'s>>,
    },
    List {
        span: Span,
        elements: Vec<Expr<'s>>,
    },
    Map {
        span: Span,
        entries: Vec<(Expr<'s>, Expr<'s>)>,
    },
    Error {
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LitKind {
    String,
    Number,
    Bool,
    Null,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse<'s> {
    pub source: &'s str,
    pub lexed: Lexed,
    pub module: Module<'s>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Parse<'_> {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.diagnostics.is_empty() && self.lexed.diagnostics.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticToken {
    pub kind: &'static str,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SemanticTokenization {
    pub tokens: Vec<SemanticToken>,
    pub diagnostics: Vec<Diagnostic>,
}

impl SemanticTokenization {
    #[must_use]
    pub fn to_host(self) -> HostTokenization {
        let valid = self.diagnostics.is_empty();
        HostTokenization {
            tokens: self
                .tokens
                .into_iter()
                .map(|t| HostToken {
                    kind: Cow::Borrowed(t.kind),
                    span: HostSpan::from(t.span),
                    error: t.kind == "error" || t.kind == "invalid",
                })
                .collect(),
            diagnostics: self
                .diagnostics
                .into_iter()
                .map(HostDiagnostic::from_diagnostic)
                .collect(),
            valid,
        }
    }
}

// ─── Lex ─────────────────────────────────────────────────────────────────────

#[inline]
fn contains_ascii_case_insensitive(list: &[&str], needle: &str) -> bool {
    let len = needle.len();
    list.iter()
        .any(|&item| item.len() == len && item.eq_ignore_ascii_case(needle))
}

#[inline]
fn classify_linear(profile: &FullProfile, text: &str) -> SyntaxKind {
    if contains_ascii_case_insensitive(profile.keywords, text) {
        SyntaxKind::Keyword
    } else if contains_ascii_case_insensitive(profile.types, text) {
        SyntaxKind::TypeIdent
    } else {
        SyntaxKind::Identifier
    }
}

#[must_use]
pub fn lex_full(source: &str, profile: &FullProfile) -> Lexed {
    if source.len() >= 96 {
        let lookup = KeywordLookup::from_profile(profile);
        lex_full_with_lookup(source, profile, Some(&lookup))
    } else {
        lex_full_with_lookup(source, profile, None)
    }
}

#[must_use]
pub fn lex_full_with_lookup(
    source: &str,
    profile: &FullProfile,
    lookup: Option<&KeywordLookup>,
) -> Lexed {
    let bytes = source.as_bytes();
    let mut out = Lexed {
        tokens: Vec::with_capacity(bytes.len() / 4),
        diagnostics: Vec::new(),
    };
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            push_lex(&mut out, SyntaxKind::Whitespace, start, i);
            continue;
        }
        if let Some(m) = profile.line_comment {
            let mb = m.as_bytes();
            if bytes[i..].starts_with(mb) {
                let start = i;
                i += mb.len();
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                push_lex(&mut out, SyntaxKind::LineComment, start, i);
                continue;
            }
        }
        if profile.hash_line_comment && b == b'#' {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            push_lex(&mut out, SyntaxKind::LineComment, start, i);
            continue;
        }
        if let Some((open, close)) = profile.block_comment {
            let o = open.as_bytes();
            let c = close.as_bytes();
            if bytes[i..].starts_with(o) {
                let start = i;
                i += o.len();
                let mut closed = false;
                while i < bytes.len() {
                    if bytes[i..].starts_with(c) {
                        i += c.len();
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                push_lex(&mut out, SyntaxKind::BlockComment, start, i);
                if !closed {
                    out.diagnostics.push(Diagnostic::new(
                        Span::new(start, i),
                        "unclosed-block-comment",
                        "Unclosed block comment",
                    ));
                }
                continue;
            }
        }
        if profile.triple_strings
            && (bytes[i..].starts_with(b"'''") || bytes[i..].starts_with(b"\"\"\""))
        {
            let q = if bytes[i] == b'\'' { b"'''" } else { b"\"\"\"" };
            let start = i;
            i += 3;
            let mut closed = false;
            while i + 2 < bytes.len() {
                if bytes[i..].starts_with(q) {
                    i += 3;
                    closed = true;
                    break;
                }
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                i += 1;
            }
            if !closed {
                i = bytes.len();
                out.diagnostics.push(Diagnostic::new(
                    Span::new(start, i),
                    "unclosed-string",
                    "Unclosed triple-quoted string",
                ));
            }
            push_lex(&mut out, SyntaxKind::StringLit, start, i);
            continue;
        }
        // Paired backticks shield embedded delimiters in templates, SQL names,
        // and shell commands. Haskell infix names keep their token structure.
        if (profile.dollar_ident || profile.backtick_strings)
            && b == b'`'
            && let Some(close) = find_backtick_end(bytes, i + 1)
        {
            push_lex(&mut out, SyntaxKind::StringLit, i, close);
            i = close;
            continue;
        }
        if b == b'"' || b == b'\'' {
            let q = b;
            let start = i;
            i += 1;
            let mut closed = false;
            while i < bytes.len() {
                if bytes[i] == q {
                    i += 1;
                    closed = true;
                    break;
                }
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'\n' {
                    break;
                }
                i += 1;
            }
            push_lex(&mut out, SyntaxKind::StringLit, start, i);
            if !closed {
                out.diagnostics.push(Diagnostic::new(
                    Span::new(start, i),
                    "unclosed-string",
                    "Unclosed string",
                ));
            }
            continue;
        }
        if b.is_ascii_digit() || (b == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit())
        {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric()
                    || bytes[i] == b'_'
                    || bytes[i] == b'.'
                    || ((bytes[i] == b'+' || bytes[i] == b'-')
                        && matches!(bytes[i - 1], b'e' | b'E' | b'p' | b'P')))
            {
                i += 1;
            }
            push_lex(&mut out, SyntaxKind::NumberLit, start, i);
            continue;
        }
        if is_ident_start(b, profile.dollar_ident) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i], profile.dollar_ident) {
                i += 1;
            }
            let text = &source[start..i];
            let kind = match lookup {
                Some(table) => table.classify(profile, text),
                None => classify_linear(profile, text),
            };
            push_lex(&mut out, kind, start, i);
            continue;
        }
        if let Some(len) = match_common_operator(bytes, i) {
            push_lex(&mut out, SyntaxKind::Operator, i, i + len);
            i += len;
            continue;
        }
        if b.is_ascii_punctuation() {
            push_lex(&mut out, SyntaxKind::Punctuation, i, i + 1);
            i += 1;
            continue;
        }
        let start = i;
        i += source[i..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(1);
        push_lex(&mut out, SyntaxKind::Identifier, start, i);
    }
    check_delimiters(source, &mut out);
    out
}

/// Bracket balance over the token stream.
///
/// The one structural claim the shared pipeline can make for every profiled
/// language without modeling its grammar: a `{`, `(` or `[` that is never
/// closed, or a closer with no opener, is broken in all of them. Strings and
/// comments are single tokens by the time this runs, so a bracket character
/// inside one cannot reach here. The parser below reports nothing of its own
/// (see [`Parser::expect_text`]), which leaves this pass as the recovery signal.
fn check_delimiters(source: &str, out: &mut Lexed) {
    // `(group, opener span)`; a mismatched closer drops its opener so one
    // mistake does not cascade into a diagnostic per remaining token.
    let mut stack: Vec<(usize, Span)> = Vec::new();
    let mut found: Vec<Diagnostic> = Vec::new();
    for token in &out.tokens {
        if token.kind != SyntaxKind::Punctuation {
            continue;
        }
        let Some(text) = token.span.slice(source) else {
            continue;
        };
        let Some((group, is_open)) = delimiter_role(text) else {
            continue;
        };
        if is_open {
            stack.push((group, token.span));
            continue;
        }
        if stack.last().is_some_and(|(open, _)| *open == group) {
            stack.pop();
        } else {
            found.push(Diagnostic::new(
                token.span,
                "stray-delimiter",
                "Closing delimiter with no matching opener",
            ));
            stack.pop();
        }
    }
    for (_, span) in stack {
        found.push(Diagnostic::new(
            span,
            "unclosed-delimiter",
            "Delimiter opened here is never closed",
        ));
    }
    found.sort_by_key(|diagnostic| diagnostic.span.start);
    out.diagnostics.extend(found);
}

/// Which bracket group a punctuation string belongs to, and whether it opens it.
#[inline]
fn delimiter_role(text: &str) -> Option<(usize, bool)> {
    match text.as_bytes() {
        b"{" => Some((0, true)),
        b"(" => Some((1, true)),
        b"[" => Some((2, true)),
        b"}" => Some((0, false)),
        b")" => Some((1, false)),
        b"]" => Some((2, false)),
        _ => None,
    }
}

/// End offset just past the next unescaped backtick, or `None` if there is none.
fn find_backtick_end(bytes: &[u8], mut i: usize) -> Option<usize> {
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'`' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

#[inline]
fn push_lex(out: &mut Lexed, kind: SyntaxKind, start: usize, end: usize) {
    if end > start {
        out.tokens.push(LexToken {
            kind,
            span: Span::new(start, end),
        });
    }
}

#[inline]
fn is_ident_start(b: u8, dollar: bool) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || (dollar && b == b'$')
}

#[inline]
fn is_ident_continue(b: u8, dollar: bool) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || (dollar && b == b'$')
}

// ─── Parser ──────────────────────────────────────────────────────────────────

struct Parser<'s, 't> {
    source: &'s str,
    tokens: &'t [LexToken],
    pos: usize,
    profile: FullProfile,
}

impl<'s, 't> Parser<'s, 't> {
    fn new(source: &'s str, lexed: &'t Lexed, profile: FullProfile) -> Self {
        let mut parser = Self {
            source,
            tokens: &lexed.tokens,
            pos: 0,
            profile,
        };
        parser.skip_trivia();
        parser
    }

    #[inline]
    fn skip_trivia(&mut self) {
        while self.pos < self.tokens.len() && self.tokens[self.pos].kind.is_trivia() {
            self.pos += 1;
        }
    }

    #[inline]
    fn peek(&self) -> Option<LexToken> {
        self.tokens.get(self.pos).copied()
    }

    #[inline]
    fn bump(&mut self) -> Option<LexToken> {
        let t = self.peek()?;
        self.pos += 1;
        self.skip_trivia();
        Some(t)
    }

    #[inline]
    fn at_text(&self, text: &str) -> bool {
        self.peek()
            .and_then(|t| t.span.slice(self.source))
            .is_some_and(|s| s == text)
    }

    #[inline]
    fn at_kw(&self, kw: &str) -> bool {
        self.peek()
            .and_then(|t| t.span.slice(self.source))
            .is_some_and(|s| s.eq_ignore_ascii_case(kw))
    }

    #[inline]
    fn at_kind(&self, kind: SyntaxKind) -> bool {
        self.peek().is_some_and(|t| t.kind == kind)
    }

    #[inline]
    fn eat_text(&mut self, text: &str) -> bool {
        if self.at_text(text) {
            self.bump();
            true
        } else {
            false
        }
    }

    #[inline]
    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.at_kw(kw) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// Consume `text` when it is next. A missing token is never reported here:
    /// an unrecognized construct is evidence of a grammar this shared parser
    /// does not model, not evidence that the document is broken. Provably broken
    /// delimiters come from [`check_delimiters`], over the token stream.
    #[inline]
    fn expect_text(&mut self, text: &str) -> Option<LexToken> {
        if self.at_text(text) {
            self.bump()
        } else {
            None
        }
    }

    #[inline]
    fn text_of(&self, t: LexToken) -> &'s str {
        t.span.slice(self.source).unwrap_or("")
    }

    fn parse_decl_body(&mut self, fallback_span: Span, eat_semi: bool) -> Block<'s> {
        if self.at_text("{") {
            self.parse_block()
        } else if self.profile.soft_indent_blocks && self.eat_text(":") {
            self.parse_suite_after_colon(fallback_span.start)
        } else {
            if eat_semi {
                self.eat_text(";");
            }
            Block {
                span: fallback_span,
                stmts: Vec::new(),
            }
        }
    }

    fn parse_control_body(&mut self, fallback_span: Span) -> Block<'s> {
        if self.at_text("{") {
            self.parse_block()
        } else if self.profile.soft_indent_blocks && self.eat_text(":") {
            self.parse_suite_after_colon(fallback_span.start)
        } else {
            Block {
                span: fallback_span,
                stmts: vec![self.parse_stmt()],
            }
        }
    }

    fn parse_module(&mut self) -> Module<'s> {
        let start = self.peek().map(|t| t.span.start).unwrap_or(0);
        let mut items = Vec::new();
        while self.peek().is_some() {
            items.push(self.parse_item());
        }
        let end = items
            .last()
            .map(|i| item_span(i).end)
            .unwrap_or(self.source.len());
        Module {
            span: Span::new(start, end),
            items,
        }
    }

    fn parse_item(&mut self) -> Item<'s> {
        if self.at_kw("function")
            || self.at_kw("fn")
            || self.at_kw("def")
            || self.at_kw("func")
            || self.at_kw("fun")
            || self.at_kw("sub")
            || self.at_kw("procedure")
        {
            return self.parse_function();
        }
        if self.at_kw("class")
            || self.at_kw("struct")
            || self.at_kw("interface")
            || self.at_kw("trait")
            || self.at_kw("type")
            || self.at_kw("enum")
            || self.at_kw("module")
            || self.at_kw("package")
        {
            return self.parse_class_like();
        }
        Item::Statement(self.parse_stmt())
    }

    fn parse_function(&mut self) -> Item<'s> {
        let start_tok = self.bump().unwrap();
        let mut name = None;
        let mut name_span = None;
        if self.at_kind(SyntaxKind::Identifier) || self.at_kind(SyntaxKind::TypeIdent) {
            let t = self.bump().unwrap();
            name = Some(self.text_of(t));
            name_span = Some(t.span);
        }
        // skip param list
        if self.eat_text("(") {
            self.skip_balanced('(', ')');
        }
        // optional return type junk
        if self.eat_text("->") || self.eat_text(":") {
            while let Some(t) = self.peek() {
                let txt = self.text_of(t);
                if txt == "{" || txt == ":" || txt == ";" || t.kind == SyntaxKind::Keyword {
                    break;
                }
                if matches!(
                    t.kind,
                    SyntaxKind::Identifier
                        | SyntaxKind::TypeIdent
                        | SyntaxKind::Operator
                        | SyntaxKind::Punctuation
                ) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        let body = self.parse_decl_body(start_tok.span, true);
        let span = Span::new(start_tok.span.start, body.span.end);
        Item::Function {
            span,
            name,
            name_span,
            body,
        }
    }

    fn parse_class_like(&mut self) -> Item<'s> {
        let start_tok = self.bump().unwrap();
        let mut name = None;
        let mut name_span = None;
        if self.at_kind(SyntaxKind::Identifier) || self.at_kind(SyntaxKind::TypeIdent) {
            let t = self.bump().unwrap();
            name = Some(self.text_of(t));
            name_span = Some(t.span);
        }
        // skip heritage / generics loosely
        while self.peek().is_some() && !self.at_text("{") && !self.at_text(":") {
            if self.at_text(";") {
                break;
            }
            self.bump();
        }
        let body = self.parse_decl_body(start_tok.span, true);
        Item::Class {
            span: Span::new(start_tok.span.start, body.span.end),
            name,
            name_span,
            body,
        }
    }

    fn parse_suite_after_colon(&mut self, start: usize) -> Block<'s> {
        let mut stmts = Vec::new();
        while self.peek().is_some() {
            if self.at_kw("def")
                || self.at_kw("class")
                || self.at_kw("fn")
                || self.at_kw("function")
            {
                break;
            }
            stmts.push(self.parse_stmt());
            if stmts.len() > 64 {
                break;
            }
        }
        let end = stmts.last().map(|s| stmt_span(s).end).unwrap_or(start);
        Block {
            span: Span::new(start, end),
            stmts,
        }
    }

    fn parse_block(&mut self) -> Block<'s> {
        let open = self.expect_text("{");
        let start = open.map(|t| t.span.start).unwrap_or(0);
        let mut stmts = Vec::new();
        while self.peek().is_some() && !self.at_text("}") {
            stmts.push(self.parse_stmt());
        }
        let close = self.expect_text("}");
        let end = close
            .map(|t| t.span.end)
            .or_else(|| stmts.last().map(|s| stmt_span(s).end))
            .unwrap_or(start);
        Block {
            span: Span::new(start, end),
            stmts,
        }
    }

    fn parse_stmt(&mut self) -> Stmt<'s> {
        if self.at_kw("return") {
            let t = self.bump().unwrap();
            let value = if self.peek().is_some()
                && !self.at_text(";")
                && !self.at_text("}")
                && !self.at_kw("else")
            {
                Some(self.parse_expr(0))
            } else {
                None
            };
            self.eat_text(";");
            let end = value
                .as_ref()
                .map(|e| expr_span(e).end)
                .unwrap_or(t.span.end);
            return Stmt::Return {
                span: Span::new(t.span.start, end),
                value,
            };
        }
        if self.at_kw("if") {
            return self.parse_if();
        }
        if self.at_kw("while") {
            let t = self.bump().unwrap();
            let cond = if self.eat_text("(") {
                let c = self.parse_expr(0);
                self.expect_text(")");
                Some(c)
            } else {
                Some(self.parse_expr(0))
            };
            let body = self.parse_control_body(t.span);
            return Stmt::While {
                span: Span::new(t.span.start, body.span.end),
                cond,
                body,
            };
        }
        if self.at_kw("for") || self.at_kw("foreach") {
            let t = self.bump().unwrap();
            if self.eat_text("(") {
                self.skip_balanced('(', ')');
            } else {
                while self.peek().is_some() && !self.at_text("{") && !self.at_text(":") {
                    self.bump();
                }
            }
            let body = self.parse_decl_body(t.span, false);
            return Stmt::For {
                span: Span::new(t.span.start, body.span.end),
                body,
            };
        }
        if self.at_kw("let")
            || self.at_kw("var")
            || self.at_kw("const")
            || self.at_kw("val")
            || self.at_kw("mut")
            || self.at_kw("final")
            || self.at_kw("dim")
            || self.at_kw("local")
            || self.at_kw("my")
        {
            let t = self.bump().unwrap();
            let mut name = None;
            let mut name_span = None;
            if self.at_kind(SyntaxKind::Identifier) {
                let n = self.bump().unwrap();
                name = Some(self.text_of(n));
                name_span = Some(n.span);
            }
            let value = if self.eat_text("=") || self.eat_text(":=") {
                Some(self.parse_expr(0))
            } else {
                None
            };
            self.eat_text(";");
            let end = value
                .as_ref()
                .map(|e| expr_span(e).end)
                .or(name_span.map(|s| s.end))
                .unwrap_or(t.span.end);
            return Stmt::Declaration {
                span: Span::new(t.span.start, end),
                name,
                name_span,
                value,
            };
        }

        if self.peek().is_none() {
            return Stmt::Error {
                span: Span::new(self.source.len(), self.source.len()),
            };
        }
        let expr = self.parse_expr(0);
        self.eat_text(";");
        Stmt::Expr {
            span: expr_span(&expr),
            expr,
        }
    }

    fn parse_if(&mut self) -> Stmt<'s> {
        let t = self.bump().unwrap();
        let cond = if self.eat_text("(") {
            let c = self.parse_expr(0);
            self.expect_text(")");
            Some(c)
        } else {
            Some(self.parse_expr(0))
        };
        let then_block = self.parse_control_body(t.span);
        let else_block = if self.eat_kw("else") {
            if self.at_kw("if") {
                Some(Block {
                    span: then_block.span,
                    stmts: vec![self.parse_if()],
                })
            } else {
                Some(self.parse_control_body(t.span))
            }
        } else {
            None
        };
        let end = else_block
            .as_ref()
            .map(|b| b.span.end)
            .unwrap_or(then_block.span.end);
        Stmt::If {
            span: Span::new(t.span.start, end),
            cond,
            then_block,
            else_block,
        }
    }

    fn parse_expr(&mut self, min_bp: u8) -> Expr<'s> {
        let mut lhs = self.parse_prefix();
        while let Some(op) = self.peek() {
            let text = self.text_of(op);
            let Some((lbp, rbp)) = infix_bp(text) else {
                break;
            };
            if lbp < min_bp {
                break;
            }
            let op_tok = self.bump().unwrap();
            if text == "(" {
                let mut args = Vec::new();
                if !self.at_text(")") {
                    loop {
                        args.push(self.parse_expr(0));
                        if self.eat_text(",") {
                            continue;
                        }
                        break;
                    }
                }
                let close = self.expect_text(")");
                let end = close.map(|t| t.span.end).unwrap_or(op_tok.span.end);
                lhs = Expr::Call {
                    span: Span::new(expr_span(&lhs).start, end),
                    callee: Box::new(lhs),
                    args,
                };
                continue;
            }
            if text == "[" {
                let index = self.parse_expr(0);
                let close = self.expect_text("]");
                let end = close.map(|t| t.span.end).unwrap_or(op_tok.span.end);
                lhs = Expr::Index {
                    span: Span::new(expr_span(&lhs).start, end),
                    object: Box::new(lhs),
                    index: Box::new(index),
                };
                continue;
            }
            if text == "." || text == "->" || text == "::" {
                let mut field = None;
                let mut field_span = None;
                if self.at_kind(SyntaxKind::Identifier)
                    || self.at_kind(SyntaxKind::TypeIdent)
                    || self.at_kind(SyntaxKind::Keyword)
                {
                    let f = self.bump().unwrap();
                    field = Some(self.text_of(f));
                    field_span = Some(f.span);
                }
                let end = field_span.map(|s| s.end).unwrap_or(op_tok.span.end);
                lhs = Expr::Member {
                    span: Span::new(expr_span(&lhs).start, end),
                    object: Box::new(lhs),
                    field,
                    field_span,
                };
                continue;
            }
            let rhs = self.parse_expr(rbp);
            lhs = Expr::Binary {
                span: Span::new(expr_span(&lhs).start, expr_span(&rhs).end),
                left: Box::new(lhs),
                op_span: op_tok.span,
                right: Box::new(rhs),
            };
        }
        lhs
    }

    fn parse_prefix(&mut self) -> Expr<'s> {
        let Some(t) = self.peek() else {
            return Expr::Error {
                span: Span::new(self.source.len(), self.source.len()),
            };
        };
        let text = self.text_of(t);
        if text == "(" {
            self.bump();
            let expr = self.parse_expr(0);
            let close = self.expect_text(")");
            let end = close.map(|c| c.span.end).unwrap_or(expr_span(&expr).end);
            return Expr::Paren {
                span: Span::new(t.span.start, end),
                expr: Box::new(expr),
            };
        }
        if text == "[" {
            self.bump();
            let mut elements = Vec::new();
            if !self.at_text("]") {
                loop {
                    elements.push(self.parse_expr(0));
                    if self.eat_text(",") {
                        continue;
                    }
                    break;
                }
            }
            let close = self.expect_text("]");
            let end = close.map(|c| c.span.end).unwrap_or(t.span.end);
            return Expr::List {
                span: Span::new(t.span.start, end),
                elements,
            };
        }
        if text == "{" {
            self.bump();
            let mut entries = Vec::new();
            if !self.at_text("}") {
                loop {
                    if self.at_text("}") {
                        break;
                    }
                    let k = self.parse_expr(0);
                    if !self.eat_text(":") && !self.eat_text("=>") {
                        break;
                    }
                    let v = self.parse_expr(0);
                    entries.push((k, v));
                    if !self.eat_text(",") {
                        break;
                    }
                }
            }
            let close = self.expect_text("}");
            let end = close.map(|c| c.span.end).unwrap_or(t.span.end);
            return Expr::Map {
                span: Span::new(t.span.start, end),
                entries,
            };
        }
        if text == "-"
            || text == "!"
            || text == "~"
            || text.eq_ignore_ascii_case("not")
            || text == "++"
            || text == "--"
        {
            let op = self.bump().unwrap();
            let expr = self.parse_prefix();
            return Expr::Unary {
                span: Span::new(op.span.start, expr_span(&expr).end),
                op_span: op.span,
                expr: Box::new(expr),
            };
        }
        match t.kind {
            SyntaxKind::Identifier | SyntaxKind::TypeIdent | SyntaxKind::Keyword => {
                let t = self.bump().unwrap();
                let name = self.text_of(t);
                let lit = if name.eq_ignore_ascii_case("true") || name.eq_ignore_ascii_case("false")
                {
                    Some(LitKind::Bool)
                } else if name.eq_ignore_ascii_case("null")
                    || name.eq_ignore_ascii_case("none")
                    || name.eq_ignore_ascii_case("nil")
                    || name == "undefined"
                {
                    Some(LitKind::Null)
                } else {
                    None
                };
                if let Some(kind) = lit {
                    Expr::Literal {
                        span: t.span,
                        kind,
                        text: name,
                    }
                } else {
                    Expr::Ident { span: t.span, name }
                }
            }
            SyntaxKind::StringLit => {
                let t = self.bump().unwrap();
                Expr::Literal {
                    span: t.span,
                    kind: LitKind::String,
                    text: self.text_of(t),
                }
            }
            SyntaxKind::NumberLit => {
                let t = self.bump().unwrap();
                Expr::Literal {
                    span: t.span,
                    kind: LitKind::Number,
                    text: self.text_of(t),
                }
            }
            // Recovery, not an error: this token starts a construct the shared
            // grammar does not model, so consume it and keep going.
            _ => {
                let t = self.bump().unwrap();
                Expr::Error { span: t.span }
            }
        }
    }

    fn skip_balanced(&mut self, open: char, close: char) {
        let mut depth = 1i32;
        while let Some(t) = self.peek() {
            let text = self.text_of(t);
            if text.starts_with(open) && text.len() == 1 {
                depth += 1;
            } else if text.starts_with(close) && text.len() == 1 {
                depth -= 1;
                self.bump();
                if depth == 0 {
                    return;
                }
                continue;
            }
            self.bump();
        }
    }
}

fn infix_bp(op: &str) -> Option<(u8, u8)> {
    Some(match op {
        "||" | "or" | "OR" | "??" => (1, 2),
        "&&" | "and" | "AND" => (3, 4),
        "==" | "!=" | "===" | "!==" | "<" | ">" | "<=" | ">=" => (5, 6),
        "|" | "^" | "&" => (7, 8),
        "<<" | ">>" => (9, 10),
        "+" | "-" => (11, 12),
        "*" | "/" | "%" | "**" => (13, 14),
        "(" | "[" | "." | "->" | "::" => (18, 19),
        "=" | "+=" | "-=" | "*=" | "/=" | "%=" => (0, 0),
        _ => return None,
    })
}

fn expr_span<'s>(e: &Expr<'s>) -> Span {
    match e {
        Expr::Ident { span, .. }
        | Expr::Literal { span, .. }
        | Expr::Call { span, .. }
        | Expr::Binary { span, .. }
        | Expr::Unary { span, .. }
        | Expr::Member { span, .. }
        | Expr::Index { span, .. }
        | Expr::Paren { span, .. }
        | Expr::List { span, .. }
        | Expr::Map { span, .. }
        | Expr::Error { span } => *span,
    }
}

fn stmt_span<'s>(s: &Stmt<'s>) -> Span {
    match s {
        Stmt::Expr { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::For { span, .. }
        | Stmt::Declaration { span, .. }
        | Stmt::Error { span } => *span,
    }
}

fn item_span<'s>(i: &Item<'s>) -> Span {
    match i {
        Item::Function { span, .. } | Item::Class { span, .. } => *span,
        Item::Statement(s) => stmt_span(s),
    }
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Full lex + parse.
#[must_use]
pub fn parse_full<'s>(source: &'s str, profile: &FullProfile) -> Parse<'s> {
    if source.len() >= 96 {
        let lookup = KeywordLookup::from_profile(profile);
        parse_full_with_lookup(source, profile, Some(&lookup))
    } else {
        parse_full_with_lookup(source, profile, None)
    }
}

/// Full lex + parse using a precomputed [`KeywordLookup`].
#[must_use]
pub fn parse_full_with_lookup<'s>(
    source: &'s str,
    profile: &FullProfile,
    lookup: Option<&KeywordLookup>,
) -> Parse<'s> {
    let lexed = lex_full_with_lookup(source, profile, lookup);
    let module = {
        let mut parser = Parser::new(source, &lexed, *profile);
        parser.parse_module()
    };
    // Every diagnostic in the shared pipeline comes from the lex + delimiter
    // pass; the parser recovers silently. See [`Parser::expect_text`].
    let diagnostics = lexed.diagnostics.clone();
    Parse {
        source,
        lexed,
        module,
        diagnostics,
    }
}

fn build_semantic_tokens(parse: &Parse<'_>) -> Vec<SemanticToken> {
    let mut name_spans: Vec<(Span, &'static str)> = Vec::new();
    collect_names(&parse.module, &mut name_spans);

    let mut tokens = Vec::with_capacity(parse.lexed.tokens.len());
    let mut cursor = 0usize;
    for t in &parse.lexed.tokens {
        let mut kind = t.kind.as_str();
        if matches!(t.kind, SyntaxKind::Identifier | SyntaxKind::TypeIdent) {
            while cursor < name_spans.len() && name_spans[cursor].0.start < t.span.start {
                cursor += 1;
            }
            if cursor < name_spans.len() && name_spans[cursor].0 == t.span {
                kind = name_spans[cursor].1;
            }
        }
        tokens.push(SemanticToken { kind, span: t.span });
    }
    tokens
}

/// Semantic tokens: rewrite identifier kinds using AST context (function/class names).
#[must_use]
pub fn semantic_full(parse: &Parse<'_>) -> SemanticTokenization {
    SemanticTokenization {
        tokens: build_semantic_tokens(parse),
        diagnostics: parse.diagnostics.clone(),
    }
}

#[inline]
fn semantic_full_owned(parse: Parse<'_>) -> SemanticTokenization {
    let tokens = build_semantic_tokens(&parse);
    SemanticTokenization {
        tokens,
        diagnostics: parse.diagnostics,
    }
}

fn collect_names<'s>(module: &Module<'s>, out: &mut Vec<(Span, &'static str)>) {
    for item in &module.items {
        match item {
            Item::Function {
                name_span, body, ..
            } => {
                if let Some(s) = name_span {
                    out.push((*s, "function"));
                }
                collect_block(body, out);
            }
            Item::Class {
                name_span, body, ..
            } => {
                if let Some(s) = name_span {
                    out.push((*s, "class"));
                }
                collect_block(body, out);
            }
            Item::Statement(s) => collect_stmt(s, out),
        }
    }
}

fn collect_block<'s>(b: &Block<'s>, out: &mut Vec<(Span, &'static str)>) {
    for s in &b.stmts {
        collect_stmt(s, out);
    }
}

fn collect_stmt<'s>(s: &Stmt<'s>, out: &mut Vec<(Span, &'static str)>) {
    match s {
        Stmt::Declaration {
            name_span: Some(sp),
            ..
        } => {
            out.push((*sp, "variable"));
        }
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            collect_block(then_block, out);
            if let Some(e) = else_block {
                collect_block(e, out);
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } => collect_block(body, out),
        _ => {}
    }
}

/// Host-facing syntax layer from full lex.
#[must_use]
pub fn lex_to_host(lexed: Lexed) -> HostTokenization {
    let valid = lexed.diagnostics.is_empty();
    HostTokenization {
        tokens: lexed
            .tokens
            .into_iter()
            .map(|t| HostToken {
                kind: Cow::Borrowed(t.kind.as_str()),
                span: HostSpan::from(t.span),
                error: t.kind == SyntaxKind::Error,
            })
            .collect(),
        diagnostics: lexed
            .diagnostics
            .into_iter()
            .map(HostDiagnostic::from_diagnostic)
            .collect(),
        valid,
    }
}

/// One-shot full pipeline for host semantic layer.
#[must_use]
pub fn analyze_full_host(source: &str, profile: &FullProfile) -> HostTokenization {
    let parsed = parse_full(source, profile);
    semantic_full_owned(parsed).to_host()
}

/// Full engine capability bits. `VALIDATE` is deliberately absent: what this
/// pipeline can prove about a document is bracket balance and closed literals
/// ([`check_delimiters`], `unclosed-string`), not that the grammar was followed.
/// The parser therefore recovers in silence, so `s := "hi"` is accepted and
/// `func f( {` is rejected, but `SELECT * FROM` is accepted too. An engine that
/// rejects what its own specification forbids — the hand-written format engines
/// — unions `VALIDATE` into its own descriptor.
pub const FULL_ENGINE_CAPS: crate::Capabilities = crate::Capabilities::LEX
    .union(crate::Capabilities::PARSE)
    .union(crate::Capabilities::SEMANTIC);

// ─── Shared Table-Driven Host Engine & Macro ─────────────────────────────────

/// Table-driven host engine for a profiled wave language.
#[derive(Debug, Clone, Copy)]
pub struct FullEngine {
    descriptor: &'static LanguageDescriptor,
    profile: &'static FullProfile,
    lookup: &'static KeywordLookup,
}

impl FullEngine {
    #[must_use]
    pub const fn new(
        descriptor: &'static LanguageDescriptor,
        profile: &'static FullProfile,
        lookup: &'static KeywordLookup,
    ) -> Self {
        Self {
            descriptor,
            profile,
            lookup,
        }
    }

    #[must_use]
    pub const fn profile(&self) -> &'static FullProfile {
        self.profile
    }

    #[must_use]
    pub fn lex_raw(&self, source: &str) -> Lexed {
        lex_full_with_lookup(source, self.profile, Some(self.lookup))
    }

    #[must_use]
    pub fn parse_raw<'s>(&self, source: &'s str) -> Parse<'s> {
        parse_full_with_lookup(source, self.profile, Some(self.lookup))
    }

    #[must_use]
    pub fn tokenize_raw(&self, source: &str) -> SemanticTokenization {
        semantic_full_owned(self.parse_raw(source))
    }

    #[must_use]
    pub fn validate_raw(&self, source: &str) -> Vec<Diagnostic> {
        self.lex_raw(source).diagnostics
    }
}

impl HostLanguage for FullEngine {
    fn descriptor(&self) -> &'static LanguageDescriptor {
        self.descriptor
    }

    fn lex(&self, source: &str, opts: &HostAnalysisOptions) -> Result<HostTokenization, HostError> {
        let _ = self.check_entry(source, opts)?;
        Ok(lex_to_host(self.lex_raw(source)))
    }

    fn semantic_tokens(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<HostTokenization, HostError> {
        let _ = self.check_entry(source, opts)?;
        Ok(self.tokenize_raw(source).to_host())
    }

    fn diagnose(
        &self,
        source: &str,
        opts: &HostAnalysisOptions,
    ) -> Result<Vec<HostDiagnostic>, HostError> {
        let _ = self.check_entry(source, opts)?;
        Ok(self
            .validate_raw(source)
            .into_iter()
            .map(HostDiagnostic::from_diagnostic)
            .collect())
    }
}

/// Declare a complete profiled wave-language crate from its descriptor metadata and [`FullProfile`].
#[macro_export]
macro_rules! define_full_engine {
    (
        id: $id:expr,
        name: $name:expr,
        aliases: $aliases:expr,
        extensions: $extensions:expr,
        mime_types: $mime_types:expr,
        profile: $profile:expr $(,)?
    ) => {
        pub const LANGUAGE_ID: $crate::LanguageId = $id;

        pub static PROFILE: $crate::FullProfile = $profile;

        pub static LOOKUP: $crate::KeywordLookup = $crate::KeywordLookup::from_profile(&PROFILE);

        pub static DESCRIPTOR: $crate::LanguageDescriptor = $crate::full_descriptor(
            LANGUAGE_ID,
            $name,
            $aliases,
            $extensions,
            $mime_types,
            env!("CARGO_PKG_VERSION"),
        );

        pub type Host = $crate::FullEngine;

        pub static ENGINE: $crate::FullEngine =
            $crate::FullEngine::new(&DESCRIPTOR, &PROFILE, &LOOKUP);

        /// Lossless lexer.
        #[must_use]
        pub fn lex(source: &str) -> $crate::Lexed {
            ENGINE.lex_raw(source)
        }

        /// Recovering parse with borrowing AST.
        #[must_use]
        pub fn parse(source: &str) -> $crate::Parse<'_> {
            ENGINE.parse_raw(source)
        }

        /// Parser-aware semantic tokens.
        #[must_use]
        pub fn tokenize(source: &str) -> $crate::SemanticTokenization {
            ENGINE.tokenize_raw(source)
        }

        /// Diagnostics from the full pipeline.
        #[must_use]
        pub fn validate(source: &str) -> ::std::vec::Vec<$crate::Diagnostic> {
            ENGINE.validate_raw(source)
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use $crate::{HostAnalysisOptions, HostError, HostLanguage, InputLimits};

            #[test]
            fn lossless_lex() {
                let source = "fn main() { return 1; }";
                assert!(lex(source).is_lossless(source));
            }

            #[test]
            fn parse_smoke() {
                let source = "function f(x) { return x + 1; }";
                let p = parse(source);
                assert!(p.lexed.is_lossless(source));
                assert!(!p.module.items.is_empty() || source.is_empty());
                let sem = tokenize(source);
                assert_eq!(sem.tokens.len(), p.lexed.tokens.len());
            }

            #[test]
            fn host_enforces_limits_on_all_layers() {
                let mut opts = HostAnalysisOptions::default();
                opts.limits = InputLimits {
                    max_input_bytes: 4,
                    ..InputLimits::default()
                };
                let sample = "12345";
                assert!(matches!(
                    ENGINE.lex(sample, &opts),
                    Err(HostError::InputTooLarge { .. })
                ));
                assert!(matches!(
                    ENGINE.semantic_tokens(sample, &opts),
                    Err(HostError::InputTooLarge { .. })
                ));
                assert!(matches!(
                    ENGINE.diagnose(sample, &opts),
                    Err(HostError::InputTooLarge { .. })
                ));
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn py() -> FullProfile {
        FullProfile {
            keywords: &["def", "return", "if", "else", "class", "for", "while"],
            types: &[],
            line_comment: None,
            block_comment: None,
            hash_line_comment: true,
            dollar_ident: false,
            backtick_strings: false,
            triple_strings: true,
            soft_indent_blocks: true,
        }
    }

    #[test]
    fn lex_parse_function_js() {
        let profile = FullProfile {
            keywords: &["function", "return", "if", "else"],
            ..FullProfile::default()
        };
        let source = "function add(a, b) { return a + b; }";
        let p = parse_full(source, &profile);
        assert!(p.lexed.is_lossless(source));
        assert!(matches!(
            p.module.items.first(),
            Some(Item::Function { .. })
        ));
        let sem = semantic_full(&p);
        assert!(sem.tokens.iter().any(|t| t.kind == "function"));
    }

    #[test]
    fn python_def() {
        let source = "def f(x):\n    return x\n";
        let p = parse_full(source, &py());
        assert!(p.lexed.is_lossless(source));
        assert!(matches!(
            p.module.items.first(),
            Some(Item::Function { .. })
        ));
    }

    #[test]
    fn keyword_lookup_matches_linear_case_insensitively() {
        let profile = FullProfile {
            keywords: &["SELECT", "FROM", "WHERE", "function"],
            types: &["INTEGER", "VARCHAR", "int"],
            ..FullProfile::DEFAULT
        };
        let table = KeywordLookup::from_profile(&profile);
        for sample in [
            "select",
            "SELECT",
            "From",
            "FUNCTION",
            "integer",
            "VarChar",
            "INT",
            "unknown",
            "x",
            "very_long_identifier_name_over_sixty_four_bytes_to_test_mask_cap_behavior_12345",
        ] {
            assert_eq!(
                table.classify(&profile, sample),
                classify_linear(&profile, sample),
                "mismatch for {sample}"
            );
        }
    }
}

#[cfg(test)]
mod delimiter_tests {
    use super::*;

    fn codes(source: &str) -> Vec<(&'static str, usize)> {
        let lexed = lex_full(source, &FullProfile::default());
        lexed
            .diagnostics
            .iter()
            .map(|d| (d.code, d.span.start))
            .collect()
    }

    #[test]
    fn one_mismatched_closer_does_not_cascade() {
        // `[` closed by `)`: report the stray closer once and drop the opener, so
        // the remaining braces stay clean instead of each losing a diagnostic.
        assert_eq!(codes("let a = [f(x)) { }"), vec![("stray-delimiter", 13)]);
    }

    #[test]
    fn a_template_literal_is_one_string_span() {
        let js = FullProfile {
            dollar_ident: true,
            ..FullProfile::default()
        };
        // `x${y}` + a real newline + `z` inside one template literal.
        let source = "const t = `x${y}\nz`;";
        let lexed = lex_full(source, &js);
        let strings: Vec<Span> = lexed
            .tokens
            .iter()
            .filter(|t| t.kind == SyntaxKind::StringLit)
            .map(|t| t.span)
            .collect();
        assert_eq!(strings, vec![Span::new(10, 19)]);
        assert_eq!(lexed.diagnostics, Vec::new());
        // An unmatched backtick stays punctuation instead of eating the file.
        let open = lex_full("let t = `x;", &js);
        assert!(!open.tokens.iter().any(|t| t.kind == SyntaxKind::StringLit));
        assert_eq!(open.diagnostics, Vec::new());
        // Without `$` the backtick is not a quote (haskell infix).
        let other = lex_full("let t = `x`;", &FullProfile::default());
        assert!(!other.tokens.iter().any(|t| t.kind == SyntaxKind::StringLit));
    }

    #[test]
    fn brackets_inside_literals_are_not_counted() {
        assert_eq!(codes("let s = \"{ ( ]\""), Vec::new());
        assert_eq!(codes("// } ) ]\nlet x = 1"), Vec::new());
    }
}
