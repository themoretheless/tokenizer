//! Full multi-language engine kit: lossless lex → recovering parse → AST → semantic.
//!
//! Not a substitute for a language-spec-complete compiler front-end. It provides a
//! **uniform full pipeline** (LEX|PARSE|SEMANTIC|VALIDATE) for editor tooling across
//! wave languages, with dialect profiles.

use crate::{
    Diagnostic, HostDiagnostic, HostSpan, HostToken, HostTokenization, Span, verify_lossless_spans,
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

// ─── Profile ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct FullProfile {
    pub keywords: &'static [&'static str],
    pub types: &'static [&'static str],
    pub line_comment: Option<&'static str>,
    pub block_comment: Option<(&'static str, &'static str)>,
    pub hash_line_comment: bool,
    /// Allow `$` in identifiers.
    pub dollar_ident: bool,
    /// Python-style triple quotes.
    pub triple_strings: bool,
    /// `#` comments that are not only at BOL (shell/python already hash_line).
    pub soft_indent_blocks: bool,
}

impl Default for FullProfile {
    fn default() -> Self {
        Self {
            keywords: &[],
            types: &[],
            line_comment: Some("//"),
            block_comment: Some(("/*", "*/")),
            hash_line_comment: false,
            dollar_ident: false,
            triple_strings: false,
            soft_indent_blocks: false,
        }
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

#[must_use]
pub fn lex_full(source: &str, profile: &FullProfile) -> Lexed {
    let bytes = source.as_bytes();
    let mut out = Lexed::default();
    let mut i = 0usize;
    let kw: std::collections::HashSet<String> = profile
        .keywords
        .iter()
        .map(|k| k.to_ascii_lowercase())
        .collect();
    let ty: std::collections::HashSet<String> = profile
        .types
        .iter()
        .map(|k| k.to_ascii_lowercase())
        .collect();

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
        if b.is_ascii_digit()
            || (b == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit())
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
            let lower = text.to_ascii_lowercase();
            let kind = if kw.contains(&lower) {
                SyntaxKind::Keyword
            } else if ty.contains(&lower) {
                SyntaxKind::TypeIdent
            } else {
                SyntaxKind::Identifier
            };
            push_lex(&mut out, kind, start, i);
            continue;
        }
        if let Some(len) = match_op(bytes, i) {
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
        i += source[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        push_lex(&mut out, SyntaxKind::Identifier, start, i);
    }
    out
}

fn push_lex(out: &mut Lexed, kind: SyntaxKind, start: usize, end: usize) {
    if end > start {
        out.tokens.push(LexToken {
            kind,
            span: Span::new(start, end),
        });
    }
}

fn is_ident_start(b: u8, dollar: bool) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || (dollar && b == b'$')
}

fn is_ident_continue(b: u8, dollar: bool) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || (dollar && b == b'$')
}

fn match_op(bytes: &[u8], i: usize) -> Option<usize> {
    for op in [
        &b"..."[..],
        b"<<=",
        b">>=",
        b"===",
        b"!==",
        b"??=",
        b"**=",
        b"<<",
        b">>",
        b"<=",
        b">=",
        b"==",
        b"!=",
        b"&&",
        b"||",
        b"**",
        b"+=",
        b"-=",
        b"*=",
        b"/=",
        b"%=",
        b"&=",
        b"|=",
        b"^=",
        b"->",
        b"=>",
        b"::",
        b"++",
        b"--",
        b"??",
    ] {
        if bytes[i..].starts_with(op) {
            return Some(op.len());
        }
    }
    None
}

// ─── Parser ──────────────────────────────────────────────────────────────────

struct Parser<'s> {
    source: &'s str,
    tokens: Vec<LexToken>,
    pos: usize,
    diagnostics: Vec<Diagnostic>,
    profile: FullProfile,
}

impl<'s> Parser<'s> {
    fn new(source: &'s str, lexed: &Lexed, profile: FullProfile) -> Self {
        let tokens = lexed
            .tokens
            .iter()
            .copied()
            .filter(|t| !t.kind.is_trivia())
            .collect();
        Self {
            source,
            tokens,
            pos: 0,
            diagnostics: Vec::new(),
            profile,
        }
    }

    fn peek(&self) -> Option<LexToken> {
        self.tokens.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<LexToken> {
        let t = self.peek()?;
        self.pos += 1;
        Some(t)
    }

    fn at_text(&self, text: &str) -> bool {
        self.peek()
            .and_then(|t| t.span.slice(self.source))
            .is_some_and(|s| s == text)
    }

    fn at_kind(&self, kind: SyntaxKind) -> bool {
        self.peek().is_some_and(|t| t.kind == kind)
    }

    fn eat_text(&mut self, text: &str) -> bool {
        if self.at_text(text) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_text(&mut self, text: &str) -> Option<LexToken> {
        if self.at_text(text) {
            self.bump()
        } else {
            let span = self.peek().map(|t| t.span).unwrap_or_else(|| {
                Span::new(self.source.len().saturating_sub(1), self.source.len())
            });
            self.diagnostics.push(Diagnostic::new(
                span,
                "expected-token",
                "Expected token",
            ));
            None
        }
    }

    fn text_of(&self, t: LexToken) -> &'s str {
        t.span.slice(self.source).unwrap_or("")
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
        // function-like keywords
        if self.at_text("function")
            || self.at_text("fn")
            || self.at_text("def")
            || self.at_text("func")
            || self.at_text("fun")
        {
            return self.parse_function();
        }
        if self.at_text("class")
            || self.at_text("struct")
            || self.at_text("interface")
            || self.at_text("trait")
            || self.at_text("type")
            || self.at_text("enum")
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
            while self.peek().is_some()
                && !self.at_text("{")
                && !self.at_text(":")
                && !self.at_kind(SyntaxKind::Keyword)
            {
                if self.at_text(";") {
                    break;
                }
                // stop before body
                if self.at_text("{") {
                    break;
                }
                // type idents
                if self.at_kind(SyntaxKind::Identifier)
                    || self.at_kind(SyntaxKind::TypeIdent)
                    || self.at_kind(SyntaxKind::Operator)
                    || self.at_kind(SyntaxKind::Punctuation)
                {
                    let t = self.peek().unwrap();
                    if self.text_of(t) == "{" {
                        break;
                    }
                    if self.text_of(t) == ";" {
                        break;
                    }
                    self.bump();
                    continue;
                }
                break;
            }
        }
        let body = if self.at_text("{") {
            self.parse_block()
        } else if self.profile.soft_indent_blocks && self.eat_text(":") {
            // python-style: parse until dedent approximation — take following stmts until blank/end/def
            self.parse_suite_after_colon(start_tok.span.start)
        } else {
            self.eat_text(";");
            Block {
                span: start_tok.span,
                stmts: vec![],
            }
        };
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
        let body = if self.at_text("{") {
            self.parse_block()
        } else if self.profile.soft_indent_blocks && self.eat_text(":") {
            self.parse_suite_after_colon(start_tok.span.start)
        } else {
            self.eat_text(";");
            Block {
                span: start_tok.span,
                stmts: vec![],
            }
        };
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
            if self.at_text("def")
                || self.at_text("class")
                || self.at_text("fn")
                || self.at_text("function")
            {
                break;
            }
            // crude: stop on unexpected top-level-ish
            stmts.push(self.parse_stmt());
            if stmts.len() > 64 {
                break;
            }
        }
        let end = stmts
            .last()
            .map(|s| stmt_span(s).end)
            .unwrap_or(start);
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
        if self.at_text("return") {
            let t = self.bump().unwrap();
            let value = if self.peek().is_some()
                && !self.at_text(";")
                && !self.at_text("}")
                && !self.at_text("else")
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
        if self.at_text("if") {
            return self.parse_if();
        }
        if self.at_text("while") {
            let t = self.bump().unwrap();
            let cond = if self.eat_text("(") {
                let c = self.parse_expr(0);
                self.expect_text(")");
                Some(c)
            } else {
                Some(self.parse_expr(0))
            };
            let body = if self.at_text("{") {
                self.parse_block()
            } else if self.profile.soft_indent_blocks && self.eat_text(":") {
                self.parse_suite_after_colon(t.span.start)
            } else {
                Block {
                    span: t.span,
                    stmts: vec![self.parse_stmt()],
                }
            };
            return Stmt::While {
                span: Span::new(t.span.start, body.span.end),
                cond,
                body,
            };
        }
        if self.at_text("for") || self.at_text("foreach") {
            let t = self.bump().unwrap();
            // skip header
            if self.eat_text("(") {
                self.skip_balanced('(', ')');
            } else {
                while self.peek().is_some() && !self.at_text("{") && !self.at_text(":") {
                    self.bump();
                }
            }
            let body = if self.at_text("{") {
                self.parse_block()
            } else if self.profile.soft_indent_blocks && self.eat_text(":") {
                self.parse_suite_after_colon(t.span.start)
            } else {
                Block {
                    span: t.span,
                    stmts: vec![],
                }
            };
            return Stmt::For {
                span: Span::new(t.span.start, body.span.end),
                body,
            };
        }
        // declaration keywords
        if self.at_text("let")
            || self.at_text("var")
            || self.at_text("const")
            || self.at_text("val")
            || self.at_text("mut")
            || self.at_text("final")
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

        // bare expression / recovery
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
        let then_block = if self.at_text("{") {
            self.parse_block()
        } else if self.profile.soft_indent_blocks && self.eat_text(":") {
            self.parse_suite_after_colon(t.span.start)
        } else {
            Block {
                span: t.span,
                stmts: vec![self.parse_stmt()],
            }
        };
        let else_block = if self.eat_text("else") {
            if self.at_text("if") {
                Some(Block {
                    span: then_block.span,
                    stmts: vec![self.parse_if()],
                })
            } else if self.at_text("{") {
                Some(self.parse_block())
            } else if self.profile.soft_indent_blocks && self.eat_text(":") {
                Some(self.parse_suite_after_colon(t.span.start))
            } else {
                Some(Block {
                    span: t.span,
                    stmts: vec![self.parse_stmt()],
                })
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
        loop {
            let Some(op) = self.peek() else { break };
            let text = self.text_of(op);
            let Some((lbp, rbp)) = infix_bp(text) else {
                break;
            };
            if lbp < min_bp {
                break;
            }
            let op_tok = self.bump().unwrap();
            if text == "(" {
                // call
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
            // map-ish or block-as-error: try map entries
            self.bump();
            let mut entries = Vec::new();
            if !self.at_text("}") {
                loop {
                    if self.at_text("}") {
                        break;
                    }
                    let k = self.parse_expr(0);
                    if !self.eat_text(":") && !self.eat_text("=>") {
                        // not a map — recovery
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
        if text == "-" || text == "!" || text == "~" || text == "not" || text == "++" || text == "--"
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
                let lit = match name {
                    "true" | "True" | "TRUE" | "false" | "False" | "FALSE" => Some(LitKind::Bool),
                    "null" | "None" | "nil" | "NULL" | "undefined" => Some(LitKind::Null),
                    _ => None,
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
            _ => {
                let t = self.bump().unwrap();
                self.diagnostics.push(Diagnostic::new(
                    t.span,
                    "unexpected-token",
                    "Unexpected token in expression",
                ));
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
        "||" | "or" | "??" => (1, 2),
        "&&" | "and" => (3, 4),
        "==" | "!=" | "===" | "!==" | "<" | ">" | "<=" | ">=" => (5, 6),
        "|" | "^" | "&" => (7, 8),
        "<<" | ">>" => (9, 10),
        "+" | "-" => (11, 12),
        "*" | "/" | "%" | "**" => (13, 14),
        "(" | "[" | "." | "->" | "::" => (18, 19),
        "=" | "+=" | "-=" | "*=" | "/=" | "%=" => (0, 0), // handled as binary low
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
    let lexed = lex_full(source, profile);
    let mut parser = Parser::new(source, &lexed, *profile);
    let module = parser.parse_module();
    let mut diagnostics = lexed.diagnostics.clone();
    diagnostics.extend(parser.diagnostics);
    Parse {
        source,
        lexed,
        module,
        diagnostics,
    }
}

/// Semantic tokens: rewrite identifier kinds using AST context (function/class names).
#[must_use]
pub fn semantic_full(parse: &Parse<'_>) -> SemanticTokenization {
    let mut name_spans: Vec<(Span, &'static str)> = Vec::new();
    collect_names(&parse.module, &mut name_spans);

    let mut tokens = Vec::with_capacity(parse.lexed.tokens.len());
    for t in &parse.lexed.tokens {
        let mut kind = t.kind.as_str();
        if matches!(t.kind, SyntaxKind::Identifier | SyntaxKind::TypeIdent) {
            for (span, k) in &name_spans {
                if span.start == t.span.start && span.end == t.span.end {
                    kind = k;
                    break;
                }
            }
        }
        tokens.push(SemanticToken {
            kind,
            span: t.span,
        });
    }
    SemanticTokenization {
        tokens,
        diagnostics: parse.diagnostics.clone(),
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
        Stmt::Declaration { name_span, .. } => {
            if let Some(sp) = name_span {
                out.push((*sp, "variable"));
            }
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
    let mut sem = semantic_full(&parsed);
    // merge lex diagnostics already in parse.diagnostics
    if sem.diagnostics.is_empty() {
        sem.diagnostics = parsed.diagnostics;
    }
    sem.to_host()
}

/// Full engine capability bits (no CST/nav/visitor yet).
pub const FULL_ENGINE_CAPS: crate::Capabilities = crate::Capabilities::LEX
    .union(crate::Capabilities::PARSE)
    .union(crate::Capabilities::SEMANTIC)
    .union(crate::Capabilities::VALIDATE);

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
        assert!(matches!(p.module.items.first(), Some(Item::Function { .. })));
        let sem = semantic_full(&p);
        assert!(sem.tokens.iter().any(|t| t.kind == "function"));
    }

    #[test]
    fn python_def() {
        let source = "def f(x):\n    return x\n";
        let p = parse_full(source, &py());
        assert!(p.lexed.is_lossless(source));
        assert!(matches!(p.module.items.first(), Some(Item::Function { .. })));
    }
}
