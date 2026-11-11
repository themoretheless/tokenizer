//! Silent-recovery recursive-descent + Pratt parser over [`LexToken`] slices.

use crate::Span;

use super::{
    ast::{Block, Expr, Item, LitKind, Module, Parse, Stmt, expr_span, item_span, stmt_span},
    lexer::{LexToken, Lexed, SyntaxKind, lex_full_with_lookup},
    lookup::{KeywordLookup, contains_ascii_case_insensitive},
    profile::FullProfile,
};

const BUILTIN_FN_KWS: &[&str] = &["function", "fn", "def", "func", "fun", "sub", "procedure"];

const BUILTIN_TYPE_KWS: &[&str] = &[
    "class",
    "struct",
    "interface",
    "trait",
    "type",
    "enum",
    "module",
    "package",
];

const BUILTIN_DECL_KWS: &[&str] = &[
    "let", "var", "const", "val", "mut", "final", "dim", "local", "my",
];

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
    fn at_any_kw(&self, builtins: &[&str], extras: &[&str]) -> bool {
        let Some(t) = self.peek() else {
            return false;
        };
        let Some(s) = t.span.slice(self.source) else {
            return false;
        };
        contains_ascii_case_insensitive(builtins, s) || contains_ascii_case_insensitive(extras, s)
    }

    #[inline]
    fn at_fn_kw(&self) -> bool {
        self.at_any_kw(BUILTIN_FN_KWS, self.profile.fn_keywords)
    }

    #[inline]
    fn at_type_kw(&self) -> bool {
        self.at_any_kw(BUILTIN_TYPE_KWS, self.profile.type_keywords)
    }

    #[inline]
    fn at_decl_kw(&self) -> bool {
        self.at_any_kw(BUILTIN_DECL_KWS, self.profile.decl_keywords)
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
    /// does not model, not evidence that the document is broken.
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
        if self.at_fn_kw() {
            return self.parse_function();
        }
        if self.at_type_kw() {
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
            if self.at_fn_kw() || self.at_type_kw() {
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
        if self.at_decl_kw() {
            let t = self.bump().unwrap();
            // Handle common two-keyword bindings like `let mut x` or `local const x`.
            if self.at_kw("mut") || self.at_kw("const") {
                self.bump();
            }
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
    let diagnostics = lexed.diagnostics.clone();
    Parse {
        source,
        lexed,
        module,
        diagnostics,
    }
}
